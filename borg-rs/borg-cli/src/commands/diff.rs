//! Compare archives command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, DiffArgs};

pub async fn run(cli: &Cli, args: &DiffArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let _repo = open_repository(&repo_path).await?;

    println!("Comparing {} and {}", args.archive1, args.archive2);

    // Diff implementation would compare two archives

    Ok(())
}
