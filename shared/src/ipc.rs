//! IPC protocols for communication with and between the Lewdware supervisor and engine.

use anyhow::Result;
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, Name, ToFsName, ToNsName,
    tokio::prelude::*,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub mod control;
pub mod engine;

// So dependencies don't need their own `interprocess` dependency.
pub use interprocess::local_socket::tokio::{Listener, RecvHalf, SendHalf, Stream};

/// Extension traits
pub mod prelude {
    pub use interprocess::local_socket::tokio::prelude::*;
}

fn socket_name(id: &str) -> Result<Name<'static>> {
    if GenericNamespaced::is_supported() {
        Ok(format!("lewdware-{id}.sock").to_ns_name::<GenericNamespaced>()?)
    } else {
        let dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
        Ok(dir
            .join(format!("lewdware-{id}.sock"))
            .to_fs_name::<GenericFilePath>()?)
    }
}

pub fn bind_listener(name: Name<'static>) -> Result<Listener> {
    Ok(ListenerOptions::new()
        .name(name)
        .try_overwrite(true)
        .create_tokio()?)
}

async fn read_line(stream: &Stream) -> Result<String> {
    let mut recver = BufReader::new(stream);
    let mut line = String::new();
    let n = recver.read_line(&mut line).await?;
    if n == 0 {
        anyhow::bail!("connection closed");
    }
    Ok(line)
}

async fn write_line(stream: &Stream, line: &str) -> Result<()> {
    let mut sender = stream;
    sender.write_all(line.as_bytes()).await?;
    sender.write_all(b"\n").await?;
    Ok(())
}
