//! List archives/files command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, ListArgs};

pub async fn run(cli: &Cli, args: &ListArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let _repo = open_repository(&repo_path).await?;

    match &args.archive {
        Some(archive) => {
            println!("Listing files in archive: {}", archive);
            // List archive contents
        }
        None => {
            println!("Archives in repository:");
            // List all archives
        }
    }

    Ok(())
}
