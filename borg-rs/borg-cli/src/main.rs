//! Borg-Rust Command Line Interface
//!
//! A modern, efficient backup tool written in Rust.

mod commands;
mod output;
mod progress;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, Args};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Borg-Rust - Deduplicating backup program
#[derive(Parser, Debug)]
#[command(name = "borg")]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Repository location (can also be set via BORG_REPO env var)
    #[arg(short, long, env = "BORG_REPO", global = true, conflicts_with_all = ["local_repo", "remote_repo"])]
    repo: Option<String>,

    /// Local repository path
    #[arg(long, value_name = "PATH", global = true, conflicts_with_all = ["repo", "remote_repo"])]
    local_repo: Option<String>,

    /// Remote repository URL (s3://, webdav://, https://, sftp://, etc.)
    #[arg(long, value_name = "URL", global = true, conflicts_with_all = ["repo", "local_repo"])]
    remote_repo: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "warn", global = true)]
    log_level: String,

    /// Show progress information
    #[arg(short, long, global = true)]
    progress: bool,

    /// Output format (text, json)
    #[arg(long, default_value = "text", global = true)]
    format: String,

    /// Lock wait timeout in seconds (0 = don't wait)
    #[arg(long, default_value = "1", global = true)]
    lock_wait: u64,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialize a new repository
    Init(InitArgs),

    /// Create a new backup archive
    Create(CreateArgs),

    /// Extract files from an archive
    Extract(ExtractArgs),

    /// List repository contents or archive contents
    List(ListArgs),

    /// Show repository or archive information
    Info(InfoArgs),

    /// Delete archives
    Delete(DeleteArgs),

    /// Prune archives according to retention policy
    Prune(PruneArgs),

    /// Verify repository consistency (legacy, use 'verify' instead)
    Check(CheckArgs),

    /// Comprehensive integrity verification
    Verify(VerifyArgs),

    /// Mount an archive as a FUSE filesystem
    Mount(MountArgs),

    /// Unmount a mounted archive
    Umount(UmountArgs),

    /// Compare archives or archive with filesystem
    Diff(DiffArgs),

    /// Search for files across archives
    Search(commands::search::SearchArgs),

    /// Rename an archive
    Rename(RenameArgs),

    /// Compact repository (free unused space)
    Compact(CompactArgs),

    /// Manage repository keys
    Key(KeyArgs),

    /// Export repository as tar archive
    Export(ExportArgs),

    /// Import tar archive into repository
    Import(ImportArgs),

    /// Show configuration
    Config(ConfigArgs),

    /// Benchmark operations
    Benchmark(BenchmarkArgs),
}

#[derive(Args, Debug)]
struct InitArgs {
    /// WebDAV repository URL (http)
    #[arg(long, value_name = "URL", conflicts_with = "webdavs_url")]
    webdav_url: Option<String>,

    /// WebDAV repository URL (https)
    #[arg(long, value_name = "URL", conflicts_with = "webdav_url")]
    webdavs_url: Option<String>,

    /// WebDAV username (optional, overrides URL user)
    #[arg(long, value_name = "USER")]
    webdav_user: Option<String>,

    /// WebDAV password (optional, overrides URL password)
    #[arg(long, value_name = "PASS")]
    webdav_pass: Option<String>,

    /// Encryption mode (none, repokey, keyfile, repokey-blake2, keyfile-blake2)
    #[arg(short, long, default_value = "repokey", num_args = 0..=1, default_missing_value = "repokey")]
    encryption: String,

    /// Create parent directories as needed
    #[arg(long)]
    make_parent_dirs: bool,

    /// Storage quota (e.g., "100G")
    #[arg(long)]
    storage_quota: Option<String>,
}

#[derive(Args, Debug)]
struct CreateArgs {
    /// Archive name
    #[arg(required = true)]
    archive: String,

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

    /// Compression algorithm (none, lz4, zstd, zlib, lzma)
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

    /// Timestamp for archive (ISO format or "now")
    #[arg(long)]
    timestamp: Option<String>,

    /// Checkpoint interval in seconds
    #[arg(long, default_value = "1800")]
    checkpoint_interval: u64,

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
}

#[derive(Args, Debug)]
struct ExtractArgs {
    /// Archive name
    #[arg(required = true)]
    archive: String,

    /// Paths to extract (all if not specified)
    paths: Vec<PathBuf>,

    /// WebDAV username
    #[arg(long, value_name = "USER")]
    webdav_user: Option<String>,

    /// WebDAV password
    #[arg(long, value_name = "PASS")]
    webdav_pass: Option<String>,

    /// Destination directory
    #[arg(long, default_value = ".")]
    destination: PathBuf,

    /// Exclude paths matching pattern
    #[arg(short, long, action = clap::ArgAction::Append)]
    exclude: Vec<String>,

    /// Strip leading path components
    #[arg(long, default_value = "0")]
    strip_components: usize,

    /// Don't restore file permissions
    #[arg(long)]
    no_permissions: bool,

    /// Don't restore ownership
    #[arg(long)]
    no_ownership: bool,

    /// Don't restore timestamps
    #[arg(long)]
    no_timestamps: bool,

    /// Don't restore ACLs
    #[arg(long)]
    no_acls: bool,

    /// Don't restore xattrs
    #[arg(long)]
    no_xattrs: bool,

    /// Dry run
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Print extracted files
    #[arg(long)]
    list: bool,

    /// Output to stdout (for single file)
    #[arg(long)]
    stdout: bool,

    /// Overwrite existing files
    #[arg(long)]
    overwrite: bool,
}

#[derive(Args, Debug)]
struct ListArgs {
    /// Archive name (list repository if not specified)
    archive: Option<String>,

    /// Paths to list (all if not specified)
    paths: Vec<PathBuf>,

    /// WebDAV username
    #[arg(long, value_name = "USER")]
    webdav_user: Option<String>,

    /// WebDAV password
    #[arg(long, value_name = "PASS")]
    webdav_pass: Option<String>,

    /// Short format (paths only)
    #[arg(long)]
    short: bool,

    /// JSON format
    #[arg(long)]
    json: bool,

    /// Show more details
    #[arg(long)]
    format: Option<String>,

    /// Sort by (name, size, timestamp)
    #[arg(long)]
    sort_by: Option<String>,

    /// First N items
    #[arg(long)]
    first: Option<usize>,

    /// Last N items
    #[arg(long)]
    last: Option<usize>,

    /// Pattern to match
    #[arg(short, long)]
    pattern: Option<String>,

    /// Exclude pattern
    #[arg(short, long, action = clap::ArgAction::Append)]
    exclude: Vec<String>,
}

#[derive(Args, Debug)]
struct InfoArgs {
    /// Archive name (show repository info if not specified)
    archive: Option<String>,

    /// JSON output
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct DeleteArgs {
    /// Archives to delete
    #[arg(required = true)]
    archives: Vec<String>,

    /// Don't ask for confirmation
    #[arg(short, long)]
    force: bool,

    /// Print statistics
    #[arg(short, long)]
    stats: bool,

    /// Don't actually delete
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Delete all archives matching prefix
    #[arg(long)]
    prefix: Option<String>,
}

#[derive(Args, Debug)]
struct PruneArgs {
    /// Keep archives within time span (e.g., "7d", "4w")
    #[arg(long)]
    keep_within: Option<String>,

    /// Number of hourly archives to keep
    #[arg(long, short = 'H')]
    keep_hourly: Option<u32>,

    /// Number of daily archives to keep
    #[arg(long, short = 'd')]
    keep_daily: Option<u32>,

    /// Number of weekly archives to keep
    #[arg(long, short = 'w')]
    keep_weekly: Option<u32>,

    /// Number of monthly archives to keep
    #[arg(long, short = 'm')]
    keep_monthly: Option<u32>,

    /// Number of yearly archives to keep
    #[arg(long, short = 'y')]
    keep_yearly: Option<u32>,

    /// Only consider archives matching prefix
    #[arg(long)]
    prefix: Option<String>,

    /// Only consider archives matching glob pattern
    #[arg(long, short = 'a')]
    glob_archives: Option<String>,

    /// Don't actually delete
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Print statistics
    #[arg(short, long)]
    stats: bool,

    /// Print archives being kept/pruned
    #[arg(long)]
    list: bool,
}

#[derive(Args, Debug)]
struct CheckArgs {
    /// Only check repository (not archives)
    #[arg(long)]
    repository_only: bool,

    /// Only check archives (not repository)
    #[arg(long)]
    archives_only: bool,

    /// Verify data integrity
    #[arg(long)]
    verify_data: bool,

    /// Repair any issues found
    #[arg(long)]
    repair: bool,

    /// Only check first N archives
    #[arg(long)]
    first: Option<usize>,

    /// Only check last N archives
    #[arg(long)]
    last: Option<usize>,

    /// Only check archives matching prefix
    #[arg(long)]
    prefix: Option<String>,
}

#[derive(Args, Debug)]
struct VerifyArgs {
    /// Specific archive to verify (verifies all if not specified)
    archive: Option<String>,

    /// Only verify repository structure (not archives)
    #[arg(long)]
    repository_only: bool,

    /// Only verify archives (not repository chunks)
    #[arg(long)]
    archives_only: bool,

    /// Run restore test on sample files
    #[arg(long)]
    test_restore: bool,

    /// Number of sample files for restore test (default: 10)
    #[arg(long)]
    sample_files: Option<usize>,

    /// Attempt to repair issues found
    #[arg(long)]
    repair: bool,

    /// JSON output format
    #[arg(long)]
    json: bool,

    /// Only verify first N archives
    #[arg(long)]
    first: Option<usize>,

    /// Only verify last N archives
    #[arg(long)]
    last: Option<usize>,

    /// Only verify archives matching glob pattern
    #[arg(long, short = 'a')]
    glob_archives: Option<String>,
}

#[derive(Args, Debug)]
struct MountArgs {
    /// Archive to mount (or repository for all archives)
    archive: Option<String>,

    /// Mount point
    #[arg(required = true)]
    mountpoint: PathBuf,

    /// FUSE options
    #[arg(short, long)]
    options: Option<String>,

    /// Run in foreground
    #[arg(short, long)]
    foreground: bool,
}

#[derive(Args, Debug)]
struct UmountArgs {
    /// Mount point to unmount
    #[arg(required = true)]
    mountpoint: PathBuf,
}

#[derive(Args, Debug)]
struct DiffArgs {
    /// First archive
    #[arg(required = true)]
    archive1: String,

    /// Second archive
    #[arg(required = true)]
    archive2: String,

    /// Paths to compare
    paths: Vec<PathBuf>,

    /// JSON output
    #[arg(long)]
    json: bool,

    /// Sort by (path, size, timestamp)
    #[arg(long)]
    sort: Option<String>,
}

#[derive(Args, Debug)]
struct RenameArgs {
    /// Current archive name
    #[arg(required = true)]
    archive: String,

    /// New archive name
    #[arg(required = true)]
    new_name: String,
}

#[derive(Args, Debug)]
struct CompactArgs {
    /// Threshold percentage for compaction
    #[arg(long, default_value = "10")]
    threshold: u32,

    /// Clean up partial/aborted transactions
    #[arg(long)]
    cleanup_commits: bool,
}

#[derive(Args, Debug)]
struct KeyArgs {
    #[command(subcommand)]
    command: KeyCommands,
}

#[derive(Subcommand, Debug)]
enum KeyCommands {
    /// Change repository passphrase
    ChangePassphrase,
    /// Export repository key
    Export {
        /// Output file (or stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Export as QR code
        #[arg(long)]
        qr: bool,
        /// Paper key format
        #[arg(long)]
        paper: bool,
    },
    /// Import repository key
    Import {
        /// Input file
        #[arg(required = true)]
        input: PathBuf,
    },
}

#[derive(Args, Debug)]
struct ExportArgs {
    /// Archive to export
    #[arg(required = true)]
    archive: String,

    /// Output file (stdout if not specified)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Tar format (auto, PAX, GNU, USTAR)
    #[arg(long, default_value = "auto")]
    tar_format: String,
}

#[derive(Args, Debug)]
struct ImportArgs {
    /// Archive name to create
    #[arg(required = true)]
    archive: String,

    /// Input tar file (stdin if not specified)
    #[arg(short, long)]
    input: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct ConfigArgs {
    /// Show effective configuration
    #[arg(long)]
    show: bool,

    /// Configuration key to get/set
    key: Option<String>,

    /// Value to set
    value: Option<String>,
}

#[derive(Args, Debug)]
struct BenchmarkArgs {
    #[command(subcommand)]
    command: BenchmarkCommands,
}

#[derive(Subcommand, Debug)]
enum BenchmarkCommands {
    /// Benchmark CPU performance
    Cpu,
    /// Benchmark compression
    Compression {
        /// File to use for benchmark
        #[arg(required = true)]
        file: PathBuf,
    },
    /// Benchmark chunking
    Chunking {
        /// File to use for benchmark
        #[arg(required = true)]
        file: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&cli.log_level));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();

    // Execute command
    let result = match &cli.command {
        Commands::Init(args) => commands::init::run(&cli, args).await,
        Commands::Create(args) => commands::create::run(&cli, args).await,
        Commands::Extract(args) => commands::extract::run(&cli, args).await,
        Commands::List(args) => commands::list::run(&cli, args).await,
        Commands::Info(args) => commands::info::run(&cli, args).await,
        Commands::Delete(args) => commands::delete::run(&cli, args).await,
        Commands::Prune(args) => commands::prune::run(&cli, args).await,
        Commands::Check(args) => commands::check::run(&cli, args).await,
        Commands::Verify(args) => commands::verify::run(&cli, args).await,
        Commands::Mount(args) => commands::mount::run(&cli, args).await,
        Commands::Umount(args) => commands::umount::run(&cli, args).await,
        Commands::Diff(args) => commands::diff::run(&cli, args).await,
        Commands::Search(args) => commands::search::run(&cli, args).await,
        Commands::Rename(args) => commands::rename::run(&cli, args).await,
        Commands::Compact(args) => commands::compact::run(&cli, args).await,
        Commands::Key(args) => commands::key::run(&cli, args).await,
        Commands::Export(args) => commands::export::run(&cli, args).await,
        Commands::Import(args) => commands::import::run(&cli, args).await,
        Commands::Config(args) => commands::config::run(&cli, args).await,
        Commands::Benchmark(args) => commands::benchmark::run(&cli, args).await,
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        let mut source = e.source();
        while let Some(err) = source {
            eprintln!("Caused by: {}", err);
            source = err.source();
        }
        std::process::exit(1);
    }

    Ok(())
}
