//! Local-socket transport: named pipes on Windows, Unix domain sockets elsewhere. With
//! `--remote host` the same messages go through `ssh host hydra proxy` instead, to the
//! server on that machine.

use crate::protocol::{self, ClientMsg, ServerMsg};
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use interprocess::local_socket::tokio::{RecvHalf, SendHalf, Stream, prelude::*};
#[cfg(unix)]
use interprocess::local_socket::{GenericFilePath, ToFsName};
#[cfg(windows)]
use interprocess::local_socket::{GenericNamespaced, ToNsName};
use interprocess::local_socket::{ListenerOptions, Name};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

pub type Reader = FramedRead<Box<dyn tokio::io::AsyncRead + Send + Unpin>, LengthDelimitedCodec>;
pub type Writer = FramedWrite<Box<dyn tokio::io::AsyncWrite + Send + Unpin>, LengthDelimitedCodec>;
#[allow(dead_code)]
type Halves = (RecvHalf, SendHalf);

const MAX_FRAME: usize = 64 * 1024 * 1024;

/// Server identity. `HYDRA_SOCKET` picks a separate server (like `tmux -L`).
pub fn socket_id() -> String {
    let label = std::env::var("HYDRA_SOCKET").unwrap_or_else(|_| "default".into());
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".into());
    format!("hydra-{user}-{label}.sock")
}

/// Windows: a named pipe (its default security lets only this user and admins write).
/// Unix: a socket file in a folder only this user can open, so no other local user can
/// reach the daemon (an abstract socket would have no permissions at all).
fn name() -> Result<Name<'static>> {
    let id = socket_id();
    #[cfg(windows)]
    {
        Ok(id.to_ns_name::<GenericNamespaced>()?.into_owned())
    }
    #[cfg(unix)]
    {
        let path = socket_dir()?.join(id);
        Ok(path.to_fs_name::<GenericFilePath>()?.into_owned())
    }
}

/// `$XDG_RUNTIME_DIR/hydra-<uid>` (or the temp folder's), created 0700 and checked to be
/// ours and private before use.
#[cfg(unix)]
fn socket_dir() -> Result<std::path::PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    // SAFETY: getuid has no preconditions and can't fail.
    let uid = unsafe { libc::getuid() };
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("hydra-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e).with_context(|| format!("creating {}", dir.display())),
    }
    let meta = std::fs::symlink_metadata(&dir).with_context(|| format!("checking {}", dir.display()))?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        anyhow::bail!("{} isn't a private folder of yours; remove it and start hydra again", dir.display());
    }
    Ok(dir)
}

fn codec() -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder().max_frame_length(MAX_FRAME).new_codec()
}

pub fn framed(stream: Stream) -> (Reader, Writer) {
    let (r, w) = stream.split();
    (FramedRead::new(Box::new(r), codec()), FramedWrite::new(Box::new(w), codec()))
}

/// The machine whose server this talks to (`--remote`), if not this one.
pub fn remote() -> Option<String> {
    std::env::var("HYDRA_REMOTE").ok().filter(|s| !s.trim().is_empty())
}

/// What ssh said when it failed (shown instead of a bare "connection closed").
static SSH_ERR: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// `ssh host hydra proxy`, its stdio as the connection. HYDRA_SSH replaces `ssh` (e.g.
/// "ssh -p 2222"), HYDRA_REMOTE_CMD the hydra on the far side (e.g. "~/.cargo/bin/hydra").
async fn connect_remote(host: &str) -> Result<(Reader, Writer)> {
    let ssh = std::env::var("HYDRA_SSH").unwrap_or_else(|_| "ssh".into());
    let mut words = ssh.split_whitespace();
    let prog = words.next().unwrap_or("ssh").to_string();
    let remote_cmd = std::env::var("HYDRA_REMOTE_CMD").unwrap_or_else(|_| "hydra".into());
    let mut cmd = tokio::process::Command::new(&prog);
    cmd.args(words)
        .arg("-T")
        .arg(host)
        .arg(format!("{remote_cmd} proxy"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().with_context(|| format!("running {prog} (is OpenSSH installed?)"))?;
    let stdin = child.stdin.take().context("ssh stdin")?;
    let stdout = child.stdout.take().context("ssh stdout")?;
    if let Some(mut err) = child.stderr.take() {
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf).await;
            *SSH_ERR.lock().unwrap() = String::from_utf8_lossy(&buf).trim().to_string();
        });
    }
    // ssh lives as long as the connection's writing half.
    Ok((FramedRead::new(Box::new(stdout), codec()), FramedWrite::new(Box::new(SshIn { stdin, _child: child }), codec())))
}

/// ssh's stdin, holding ssh itself: dropping the connection ends it (kill_on_drop), so
/// nothing is left waiting on its pipes.
struct SshIn {
    stdin: tokio::process::ChildStdin,
    _child: tokio::process::Child,
}

impl tokio::io::AsyncWrite for SshIn {
    fn poll_write(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &[u8]) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.stdin).poll_write(cx, buf)
    }

    fn poll_flush(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stdin).poll_flush(cx)
    }

    fn poll_shutdown(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stdin).poll_shutdown(cx)
    }
}

/// `hydra proxy`, run by ssh on the far side: this machine's server (started if needed),
/// over stdin and stdout.
pub async fn proxy() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = connect().await;
    if stream.is_err() {
        spawn_daemon()?;
        for _ in 0..60 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            stream = connect().await;
            if stream.is_ok() {
                break;
            }
        }
    }
    let (mut sr, mut sw) = stream.context("couldn't reach or start the hydra server here")?.split();
    let up = async {
        let mut stdin = tokio::io::stdin();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = stdin.read(&mut buf).await?;
            if n == 0 {
                return anyhow::Ok(());
            }
            sw.write_all(&buf[..n]).await?;
        }
    };
    let down = async {
        let mut stdout = tokio::io::stdout();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = sr.read(&mut buf).await?;
            if n == 0 {
                return anyhow::Ok(());
            }
            stdout.write_all(&buf[..n]).await?;
            stdout.flush().await?;
        }
    };
    tokio::select! {
        r = up => r,
        r = down => r,
    }
}

pub async fn connect() -> Result<Stream> {
    Ok(Stream::connect(name()?).await?)
}

pub fn listen() -> Result<interprocess::local_socket::tokio::Listener> {
    // A stale socket file from a crashed daemon would make bind fail. (Callers check
    // that no daemon answers first.)
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(socket_dir()?.join(socket_id()));
    }
    ListenerOptions::new()
        .name(name()?)
        .create_tokio()
        .context("creating the daemon socket")
}

pub async fn send<T: serde::Serialize>(w: &mut Writer, msg: &T) -> Result<()> {
    w.send(protocol::encode(msg)?).await?;
    Ok(())
}

pub async fn recv_server(r: &mut Reader) -> Result<Option<ServerMsg>> {
    match r.next().await {
        Some(frame) => Ok(Some(protocol::decode(&frame?)?)),
        None => Ok(None),
    }
}

pub async fn recv_client(r: &mut Reader) -> Result<Option<ClientMsg>> {
    match r.next().await {
        Some(frame) => Ok(Some(protocol::decode(&frame?)?)),
        None => Ok(None),
    }
}

/// Connect and complete the handshake.
pub async fn open(attach: bool) -> Result<(Reader, Writer)> {
    let (mut r, mut w) = match remote() {
        Some(host) => connect_remote(&host).await?,
        None => framed(connect().await?),
    };
    send(&mut w, &ClientMsg::Hello { version: protocol::PROTOCOL_VERSION, attach }).await?;
    let first = match recv_server(&mut r).await {
        Ok(m) => m,
        Err(e) if remote().is_some() => return Err(e.context(ssh_failure())),
        Err(e) => return Err(e),
    };
    if first.is_none() && remote().is_some() {
        bail!(ssh_failure());
    }
    match first {
        Some(ServerMsg::Welcome { version }) if version == protocol::PROTOCOL_VERSION => Ok((r, w)),
        Some(ServerMsg::Welcome { version }) => bail!(
            "daemon speaks protocol v{version}, this binary v{}; run `hydra kill-server` and retry",
            protocol::PROTOCOL_VERSION
        ),
        Some(ServerMsg::Error(e)) => bail!(e),
        _ => bail!("unexpected handshake from daemon"),
    }
}

fn ssh_failure() -> String {
    // Give ssh a moment to say why.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let err = SSH_ERR.lock().unwrap().clone();
    let host = remote().unwrap_or_default();
    if err.contains("not found") || err.contains("not recognized") {
        format!("{host} has no `hydra` on its PATH for ssh; install it there, or set HYDRA_REMOTE_CMD to its full path ({err})")
    } else if err.is_empty() {
        format!("couldn't reach hydra on {host} over ssh")
    } else {
        format!("ssh {host}: {err}")
    }
}

/// Connect, starting the daemon in the background if none is running.
pub async fn open_or_spawn(attach: bool) -> Result<(Reader, Writer)> {
    // Over ssh the far side starts its own server.
    if remote().is_some() {
        return open(attach).await;
    }
    if let Ok(c) = open(attach).await {
        return Ok(c);
    }
    spawn_daemon()?;
    let mut last = None;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        match open(attach).await {
            Ok(c) => return Ok(c),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("daemon did not start")))
}

fn spawn_daemon() -> Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // A hidden console of its own, so ConPTY works and closing this window doesn't kill it.
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own session: survives the client's terminal closing and never sees its Ctrl+C.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    cmd.spawn().context("starting the daemon")?;
    Ok(())
}
