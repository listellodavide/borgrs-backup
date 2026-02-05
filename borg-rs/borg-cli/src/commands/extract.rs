//! Archive extraction command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, ExtractArgs};

use borg_core::archive::ArchiveExtractor;

pub async fn run(cli: &Cli, args: &ExtractArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let repo = open_repository(&repo_path).await?;

    let extractor = ArchiveExtractor::new(&repo);
    let stats = extractor.extract(&args.archive, &args.destination).await?;

    println!("Extracted archive: {}", args.archive);
    println!("Files: {}", stats.files_extracted);
    println!("Directories: {}", stats.dirs_extracted);
    println!("Total bytes: {}", stats.bytes_extracted);

    Ok(())
}
