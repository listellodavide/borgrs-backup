//! Rename archive command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, RenameArgs};

pub async fn run(cli: &Cli, args: &RenameArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let _repo = open_repository(&repo_path).await?;

    println!("Renaming '{}' to '{}'", args.archive, args.new_name);

    // Rename implementation

    Ok(())
}
