//! Import tar archive

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, ImportArgs};

pub async fn run(cli: &Cli, args: &ImportArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let _repo = open_repository(&repo_path).await?;

    match &args.input {
        Some(path) => println!("Importing {:?} as {}", path, args.archive),
        None => println!("Importing from stdin as {}", args.archive),
    }

    Ok(())
}
