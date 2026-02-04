//! Delete archives command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, DeleteArgs};

pub async fn run(cli: &Cli, args: &DeleteArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let _repo = open_repository(&repo_path).await?;

    for archive in &args.archives {
        if args.dry_run {
            println!("Would delete: {}", archive);
        } else {
            println!("Deleting: {}", archive);
            // Delete archive
        }
    }

    Ok(())
}
