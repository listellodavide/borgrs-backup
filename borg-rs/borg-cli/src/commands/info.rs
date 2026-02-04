//! Repository/archive info command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, InfoArgs};

pub async fn run(cli: &Cli, args: &InfoArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let _repo = open_repository(&repo_path).await?;

    match &args.archive {
        Some(archive) => {
            println!("Archive: {}", archive);
            // Show archive info
        }
        None => {
            println!("Repository: {}", repo_path);
            // Show repository info
        }
    }

    Ok(())
}
