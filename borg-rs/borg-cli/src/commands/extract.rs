//! Archive extraction command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, ExtractArgs};

use borg_core::archive::ArchiveExtractor;

pub async fn run(cli: &Cli, args: &ExtractArgs) -> Result<()> {
    let repo_path_raw = get_repo_path(cli)?;

    // Normalize WebDAV URL if credentials are provided
    let repo_path = if args.webdav_user.is_some() || args.webdav_pass.is_some() {
        crate::commands::init::normalize_webdav_url(
            &repo_path_raw,
            "webdav",
            args.webdav_user.as_deref(),
            args.webdav_pass.as_deref(),
        )?
    } else {
        repo_path_raw
    };

    let repo = open_repository(&repo_path).await?;

    let extractor = ArchiveExtractor::new(&repo);
    let stats = extractor.extract(&args.archive, &args.destination).await?;

    println!("Extracted archive: {}", args.archive);
    println!("Files: {}", stats.files_extracted);
    println!("Directories: {}", stats.dirs_extracted);
    println!("Total bytes: {}", stats.bytes_extracted);

    Ok(())
}
