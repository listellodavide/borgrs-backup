use crate::{Cli};
use crate::commands::get_passphrase;
use borg_core::archive::{ArchiveCreator, BackupProgress, default_archive_unique_name};
use borg_core::chunker::ChunkerProfile;
use borg_core::compression::{CompressionConfig, Compressor, CompressionAlgorithm, CompressionLevel};
use borg_core::exclusion::{ExclusionList, ExclusionPattern};
use borg_core::repository::Repository;
use borg_core::storage::{build_operator, parse_storage_config};
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use std::time::Duration;
use anyhow::Result;

#[derive(Args, Debug)]
pub struct CreateArgs {
    /// Manually specify archive name, otherwise a unique name is generated
    #[arg(long = "force-archive-name")]
    archive: Option<String>,

    /// Paths to back up
    #[arg(required = true)]
    paths: Vec<PathBuf>,

    /// WebDAV username
    #[arg(long, value_name = "USER")]
    webdav_user: Option<String>,

    /// WebDAV password
    #[arg(long, value_name = "PASS")]
    webdav_pass: Option<String>,

    /// Exclude paths matching pattern
    #[arg(short, long, action = clap::ArgAction::Append)]
    exclude: Vec<String>,

    /// Read exclude patterns from file
    #[arg(long, action = clap::ArgAction::Append)]
    exclude_from: Vec<PathBuf>,

    /// Exclude directories containing CACHEDIR.TAG
    #[arg(long)]
    exclude_caches: bool,

    /// Exclude directories containing specified file
    #[arg(long, action = clap::ArgAction::Append)]
    exclude_if_present: Vec<String>,

    /// Compression algorithm (none, lz4, zstd, zlib, lzma, xz)
    #[arg(short, long, default_value = "zstd")]
    compression: String,

    /// Compression level
    #[arg(long)]
    compression_level: Option<u32>,

    /// Stay in same filesystem (don't cross mount points)
    #[arg(short = 'x', long)]
    one_file_system: bool,

    /// Open and read special files
    #[arg(long)]
    read_special: bool,

    /// Store numeric user/group IDs only
    #[arg(long)]
    numeric_ids: bool,

    /// Don't store access times
    #[arg(long)]
    noatime: bool,

    /// Exclude files flagged nodump
    #[arg(long)]
    exclude_nodump: bool,

    /// Add a comment to the archive
    #[arg(long)]
    comment: Option<String>,

    /// Add tags to the archive
    #[arg(long, action = clap::ArgAction::Append)]
    tags: Option<Vec<String>>,

    /// Timestamp for archive (ISO format or "now")
    #[arg(long)]
    timestamp: Option<String>,

    /// Checkpoint interval in seconds
    #[arg(long, default_value = "1800")]
    checkpoint_interval: u64,

    /// Force a specific chunker profile (e.g., 1mb, 4mb, default)
    #[arg(long)]
    force_chunk_profile: Option<ChunkerProfile>,

    /// Dry run (don't create archive)
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Print statistics
    #[arg(short, long)]
    stats: bool,

    /// Print file list
    #[arg(long)]
    list: bool,

    /// Print files with status (A=added, M=modified, etc.)
    #[arg(long)]
    filter: Option<String>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

struct CliProgress {
    pb: ProgressBar,
    list_files: bool,
}

impl CliProgress {
    fn new(list_files: bool) -> Self {
        let pb = ProgressBar::new(0);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );
        pb.enable_steady_tick(Duration::from_millis(100));
        Self { pb, list_files }
    }
}

impl BackupProgress for CliProgress {
    fn on_file_start(&self, path: &Path) {
        if self.list_files {
            self.pb.println(format!("A {}", path.display()));
        }
        self.pb.set_message(format!("Processing: {}", path.display()));
    }

    fn on_file_complete(&self, _path: &Path, _size: u64, _chunks: usize) {
        self.pb.inc(1);
    }

    fn on_file_skipped(&self, path: &Path, reason: &str) {
        if self.list_files {
            self.pb.println(format!("S {} ({})", path.display(), reason));
        }
    }

    fn on_progress(&self, processed: u64, total: u64, filename: Option<&str>) {
        self.pb.set_length(total);
        self.pb.set_position(processed);
        if let Some(name) = filename {
             self.pb.set_message(format!("Processing: {}", name));
        }
    }

    fn on_error(&self, path: &Path, error: &str) {
        self.pb.println(format!("E {} ({})", path.display(), error));
    }
}

fn build_compressor(args: &CreateArgs) -> Result<Compressor> {
    let algo_str = &args.compression;
    let level = args.compression_level;

    let algorithm = match algo_str.to_lowercase().as_str() {
        "none" => CompressionAlgorithm::None,
        "lz4" => CompressionAlgorithm::Lz4,
        "zstd" => CompressionAlgorithm::Zstd,
        "zlib" => CompressionAlgorithm::Zlib,
        "lzma" => CompressionAlgorithm::Lzma,
        "xz" => CompressionAlgorithm::Xz,
        _ => return Err(anyhow::anyhow!("Unknown compression algorithm: {}", algo_str)),
    };

    let level_val = level.unwrap_or(3) as u8;
    let compression_level = CompressionLevel::new(level_val).unwrap_or(CompressionLevel::DEFAULT);

    let config = CompressionConfig {
        algorithm,
        level: compression_level,
        ..Default::default()
    };

    Ok(Compressor::new(config))
}

pub async fn run(cli: &Cli, args: &CreateArgs) -> Result<()> {
    let repo_path_str = super::get_repo_path(cli)?;
    let passphrase = get_passphrase("Enter passphrase: ").await?;

    let config = parse_storage_config(&repo_path_str)?;
    let operator = build_operator(config)?;

    let mut repo = Repository::open(operator, repo_path_str, Some(&passphrase)).await?;

    let mut exclusions = ExclusionList::new();
    for pattern in &args.exclude {
        exclusions.add_pattern(ExclusionPattern::glob(pattern))?;
    }

    // Handle exclude_from files
    for file in &args.exclude_from {
        if file.exists() {
             let file_exclusions = ExclusionList::from_file(file)?;
             for pattern in file_exclusions.patterns() {
                 exclusions.add_pattern(pattern.clone())?;
             }
        } else {
            eprintln!("Warning: Exclusion file not found: {}", file.display());
        }
    }

    // Handle exclude_caches
    if args.exclude_caches {
        use borg_core::exclusion::CommonExclusions;
        for pattern in CommonExclusions::caches() {
            exclusions.add_pattern(pattern)?;
        }
    }

    let progress = Box::new(CliProgress::new(args.list || args.verbose));

    let compressor = build_compressor(args)?;
    repo.set_compressor(compressor);

    let mut creator = ArchiveCreator::new(&mut repo)
        .with_exclusions(exclusions)
        .with_progress(progress);

    if let Some(profile) = args.force_chunk_profile {
        creator = creator.with_forced_chunker_profile(profile);
    }

    let archive_name = args.archive.clone().unwrap_or_else(default_archive_unique_name);

    if args.dry_run {
        println!("Dry run: would create archive '{}' from {} paths", archive_name, args.paths.len());
        return Ok(());
    }

    let archive = creator
        .create(
            &archive_name,
            &args.paths,
            args.comment.clone(),
            args.tags.clone(),
        )
        .await?;

    if args.stats {
        println!(
            "Archive '{}' created successfully. Size: {}, Compressed: {}, Dedup: {}",
            archive.metadata.name,
            humansize::format_size(archive.stats.original_size, humansize::BINARY),
            humansize::format_size(archive.stats.compressed_size, humansize::BINARY),
            humansize::format_size(archive.stats.deduplicated_size, humansize::BINARY),
        );
    } else {
        println!("Archive '{}' created successfully.", archive.metadata.name);
    }

    Ok(())
}
