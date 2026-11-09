//! One settings window at a time. A second launch -- most often the tray's "Open Lewdware" --
//! hands over to the instance already running, which brings its window forward, and exits.
//! Replaces `tauri-plugin-single-instance`.

use interprocess::local_socket::tokio::prelude::*;
use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, Name, ToFsName, ToNsName,
};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

/// How this process came out of startup.
pub enum Claim {
    /// Another instance is running and has been asked to show itself.
    Secondary,
    /// This is the only instance. A message arrives on the receiver each time a later launch
    /// asks to be shown.
    Primary(mpsc::UnboundedReceiver<()>),
}

fn socket_name() -> anyhow::Result<Name<'static>> {
    const FILE: &str = "lewdware-config.sock";
    Ok(if GenericNamespaced::is_supported() {
        FILE.to_ns_name::<GenericNamespaced>()?
    } else {
        let dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
        dir.join(FILE).to_fs_name::<GenericFilePath>()?
    })
}

pub async fn claim() -> anyhow::Result<Claim> {
    if let Ok(mut stream) = Stream::connect(socket_name()?).await {
        stream.write_all(b"show\n").await?;
        return Ok(Claim::Secondary);
    }

    // Nothing answered, so any socket file left behind is stale.
    let listener = ListenerOptions::new()
        .name(socket_name()?)
        .try_overwrite(true)
        .create_tokio()?;
    let (sender, receiver) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok(_stream) = listener.accept().await {
            if sender.send(()).is_err() {
                return;
            }
        }
    });
    Ok(Claim::Primary(receiver))
}
