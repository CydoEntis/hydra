//! Local-socket transport: named pipes on Windows, Unix domain sockets elsewhere.

use crate::protocol::{self, ClientMsg, ServerMsg};
use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use interprocess::local_socket::tokio::{RecvHalf, SendHalf, Stream, prelude::*};
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, Name, NameType, ToFsName, ToNsName,
};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

pub type Reader = FramedRead<RecvHalf, LengthDelimitedCodec>;
pub type Writer = FramedWrite<SendHalf, LengthDelimitedCodec>;

const MAX_FRAME: usize = 64 * 1024 * 1024;

/// Server identity. `HYDRA_SOCKET` picks a separate server (like `tmux -L`).
pub fn socket_id() -> String {
    let label = std::env::var("HYDRA_SOCKET").unwrap_or_else(|_| "default".into());
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".into());
    format!("hydra-{user}-{label}.sock")
}

fn name() -> Result<Name<'static>> {
    let id = socket_id();
    if GenericNamespaced::is_supported() {
        Ok(id.to_ns_name::<GenericNamespaced>()?.into_owned())
    } else {
        let path = std::env::temp_dir().join(id);
        Ok(path.to_fs_name::<GenericFilePath>()?.into_owned())
    }
}

fn codec() -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder().max_frame_length(MAX_FRAME).new_codec()
}

pub fn framed(stream: Stream) -> (Reader, Writer) {
    let (r, w) = stream.split();
    (FramedRead::new(r, codec()), FramedWrite::new(w, codec()))
}

pub async fn connect() -> Result<Stream> {
    Ok(Stream::connect(name()?).await?)
}

pub fn listen() -> Result<interprocess::local_socket::tokio::Listener> {
    // A stale socket file from a crashed daemon would make bind fail on macOS.
    if !GenericNamespaced::is_supported() {
        let path = std::env::temp_dir().join(socket_id());
        let _ = std::fs::remove_file(path);
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
    let (mut r, mut w) = framed(connect().await?);
    send(&mut w, &ClientMsg::Hello { version: protocol::PROTOCOL_VERSION, attach }).await?;
    match recv_server(&mut r).await? {
        Some(ServerMsg::Welcome { version }) if version == protocol::PROTOCOL_VERSION => Ok((r, w)),
        Some(ServerMsg::Welcome { version }) => bail!(
            "daemon speaks protocol v{version}, this binary v{}; run `hydra kill-server` and retry",
            protocol::PROTOCOL_VERSION
        ),
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
