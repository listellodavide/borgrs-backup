//! Archive restoration command

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use borg_core::archive::{ArchiveRestorer, RestoreProgress, RestoreStats};
use super::{get_repo_path, open_repository};
use crate::{Cli, RestoreArgs};

struct CliRestoreProgress {
    pb: ProgressBar,
}

impl RestoreProgress for CliRestoreProgress {
    fn on_start(&self, total_files: u64, total_bytes: u64) {
        self.pb.set_length(total_bytes);
        self.pb.set_message(format!("Restoring {} files", total_files));
    }

    fn on_file_start(&self, path: &Path, size: u64) {
        self.pb.println(format!("Restoring: {}", path.display()));
        self.pb.inc(size);
    }

    fn on_file_complete(&self, _path: &Path) {}

    fn on_error(&self, path: &Path, error: &str) {
        self.pb.println(format!("Error restoring {}: {}", path.display(), error));
    }

    fn on_finish(&self) {
        self.pb.finish_with_message("Restore complete");
    }
}

pub async fn run(cli: &Cli, args: &RestoreArgs) -> Result<()> {
    let repo_path_raw = get_repo_path(cli)?;

    let repo_path = if args.webdav_user.is_some() || args.webdav_pass.is_some() {
        let scheme = if cli.remote_repo.is_some() {
            if repo_path_raw.starts_with("https") || repo_path_raw.starts_with("webdavs") || repo_path_raw.starts_with("davs") {
                "webdavs"
            } else {
                "webdav"
            }
        } else {
            "webdav"
        };
        crate::commands::init::normalize_webdav_url(
            &repo_path_raw,
            scheme,
            args.webdav_user.as_deref(),
            args.webdav_pass.as_deref(),
        )?
    } else {
        repo_path_raw
    };

    let repo = open_repository(&repo_path).await?;
    let manifest = repo.load_manifest().await?;

    let archives_to_restore = if args.all_archives {
        manifest.archives.iter().map(|a| a.name.clone()).collect()
    } else {
        vec![args.archive.clone().context("Archive name is required when --all-archives is not used")?]
    };

    for archive_name in &archives_to_restore {
        println!("Processing archive: {}", archive_name);

        let mut restorer = ArchiveRestorer::new(&repo);

        if cli.progress {
            let pb = ProgressBar::new(0);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) {msg}")
                    .unwrap()
                    .progress_chars("#>-"),
            );
            let progress_handler = CliRestoreProgress { pb };
            restorer = restorer.with_progress(Box::new(progress_handler));
        }

        let stats: Result<RestoreStats> = if args.original_location {
            let archive = restorer.load_archive(archive_name).await?;
            if let Some(original_paths) = &archive.metadata.original_paths {
                if original_paths.len() == 1 {
                    let dest = &original_paths[0];
                    println!("Restoring to original location: {}", dest.display());
                    restorer.restore(archive_name, dest).await
                        .map_err(|e| anyhow::anyhow!(e))
                } else {
                    println!("Warning: Archive has multiple source paths. Restoring all files to current directory.");
                    restorer.restore(archive_name, &args.destination).await
                        .map_err(|e| anyhow::anyhow!(e))
                }
            } else {
                anyhow::bail!("Archive does not contain original path information");
            }
        } else {
            let dest = if args.all_archives {
                args.destination.join(archive_name)
            } else {
                args.destination.clone()
            };
            restorer.restore_paths(archive_name, &dest, &args.paths).await
                .map_err(|e| anyhow::anyhow!(e))
        };

        match stats {
            Ok(s) => {
                println!("Restored archive: {}", archive_name);
                println!("Files: {}", s.files_restored);
                println!("Directories: {}", s.dirs_restored);
                println!("Total bytes: {}", s.bytes_restored);
            }
            Err(e) => {
                eprintln!("Failed to restore archive {}: {}", archive_name, e);
            }
        }
    }

    Ok(())
}
