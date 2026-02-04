//! Export archive as tar

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, ExportArgs};

pub async fn run(cli: &Cli, args: &ExportArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let _repo = open_repository(&repo_path).await?;

    match &args.output {
        Some(path) => println!("Exporting {} to {:?}", args.archive, path),
        None => println!("Exporting {} to stdout", args.archive),
    }

    Ok(())
}
