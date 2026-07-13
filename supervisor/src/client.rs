use anyhow::Result;
use shared::ipc::Request;

use crate::Command;

pub fn run(command: Command) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let req = match command {
            Command::Status => Request::Status,
            Command::Start { mode_path, dev } => Request::StartSession { mode_path, dev },
            Command::Restart { mode_path, dev } => Request::RestartSession { mode_path, dev },
            Command::Stop => Request::StopSession,
            Command::Panic => Request::Panic,
        };

        let response = shared::ipc::request(&req).await?;
        println!("{response:#?}");
        Ok(())
    })
}
