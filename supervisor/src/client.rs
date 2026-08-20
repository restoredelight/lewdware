use anyhow::Result;
use shared::ipc::control::Request;

use crate::Command;

pub fn run(command: Command) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let req = match command {
            Command::Status => Request::Status,
            Command::Start { mode_path, dev } => Request::StartSession {
                mode_path,
                dev,
                dev_stream_id: None,
                replace: false,
            },
            Command::Restart { mode_path, dev } => Request::StartSession {
                mode_path,
                dev,
                dev_stream_id: None,
                replace: true,
            },
            Command::Stop => Request::StopSession,
            Command::Panic => Request::Panic,
        };

        let response = shared::ipc::control::request(&req).await?;
        println!("{response:#?}");
        Ok(())
    })
}
