//! Local-socket transport: named pipes on Windows, Unix domain sockets elsewhere. With
//! `--remote host` the same messages go through `ssh host seshi proxy` instead, to the
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

/// The running server is another seshi version: the two can't talk (see PROTOCOL_VERSION).
#[derive(Debug)]
pub struct OtherVersion {
    pub daemon: u32,
}

impl std::fmt::Display for OtherVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the running server is another seshi version (protocol v{}, this one v{}); `seshi kill-server` stops it, then run seshi again",
            self.daemon,
            protocol::PROTOCOL_VERSION
        )
    }
}

impl std::error::Error for OtherVersion {}

/// Where the server notes its process id, so a seshi of another version can still stop it.
pub fn pid_file() -> std::path::PathBuf {
    crate::config::data_dir().join(format!("{}.pid", socket_id()))
}

/// Server identity. `SESHI_SOCKET` picks a separate server (like `tmux -L`).
pub fn socket_id() -> String {
    let label = std::env::var("SESHI_SOCKET").unwrap_or_else(|_| "default".into());
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".into());
    format!("seshi-{user}-{label}.sock")
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

/// `$XDG_RUNTIME_DIR/seshi-<uid>` (or the temp folder's), created 0700 and checked to be
/// ours and private before use.
#[cfg(unix)]
fn socket_dir() -> Result<std::path::PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    // SAFETY: getuid has no preconditions and can't fail.
    let uid = unsafe { libc::getuid() };
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("seshi-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e).with_context(|| format!("creating {}", dir.display())),
    }
    let meta = std::fs::symlink_metadata(&dir).with_context(|| format!("checking {}", dir.display()))?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        anyhow::bail!("{} isn't a private folder of yours; remove it and start seshi again", dir.display());
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

/// Whether a receive failed on one message's contents (the connection is still fine).
pub fn is_decode_error(e: &anyhow::Error) -> bool {
    e.downcast_ref::<rmp_serde::decode::Error>().is_some()
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
    let (mut r, mut w) = framed(connect().await?);
    // A command run inside a pane says which (with the pane's secret): what it may do is
    // that pane's to say. seshi's own window is you.
    let from = (!attach)
        .then(|| {
            let term = std::env::var("SESHI_TERM_ID").ok()?.parse().ok()?;
            Some((term, std::env::var("SESHI_PANE_TOKEN").unwrap_or_default()))
        })
        .flatten();
    send(&mut w, &ClientMsg::Hello { version: protocol::PROTOCOL_VERSION, attach, from }).await?;
    match recv_server(&mut r).await? {
        Some(ServerMsg::Welcome { version }) if version == protocol::PROTOCOL_VERSION => Ok((r, w)),
        Some(ServerMsg::Welcome { version }) => Err(OtherVersion { daemon: version }.into()),
        Some(ServerMsg::Error(e)) => bail!(e),
        _ => bail!("unexpected handshake from daemon"),
    }
}

/// Connect, starting the daemon in the background if none is running.
pub async fn open_or_spawn(attach: bool) -> Result<(Reader, Writer)> {
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
        // A hidden console of its own, so ConPTY works and closing this window doesn't kill it.
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | crate::proc::CREATE_NO_WINDOW);
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

#[cfg(test)]
mod tests {
    #[test]
    fn a_bad_message_is_a_decode_error() {
        let e = crate::protocol::decode::<crate::protocol::ServerMsg>(&[0xc1, 0xff]).unwrap_err();
        assert!(super::is_decode_error(&e));
        assert!(!super::is_decode_error(&anyhow::anyhow!("connection reset")));
    }
}
