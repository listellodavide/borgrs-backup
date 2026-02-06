````markdown
# Verification and Integrity Checking

This document describes the verification and integrity checking features in Borg-RS.

## Overview

Borg-RS provides comprehensive verification capabilities to ensure the integrity of your backup data:

- **Chunk Verification**: Verify individual data chunks by checking content hashes
- **Repository Verification**: Full integrity check of all chunks in a repository
- **Archive Verification**: Verify all chunks referenced by an archive exist and are valid
- **Restore Testing**: Automated testing of archive restoration
- **Verification History**: Track verification results over time

## Quick Start

### Basic Verification

```bash
# Verify repository structure and all chunks
borg verify --repo /path/to/repo

# Verify with progress display
borg verify --repo /path/to/repo --progress

# Verify a specific archive
borg verify --repo /path/to/repo my-archive

# Only verify repository (skip archive verification)
borg verify --repo /path/to/repo --repository-only

# Only verify archives (skip chunk verification)
borg verify --repo /path/to/repo --archives-only
```

### Restore Testing

```bash
# Run restore test on sample files
borg verify --repo /path/to/repo --test-restore

# Specify number of sample files
borg verify --repo /path/to/repo --test-restore --sample-files 20
```

### JSON Output

```bash
# Output verification results as JSON
borg verify --repo /path/to/repo --json
```

## Command Reference

### `borg verify`

Comprehensive integrity verification command.

**Arguments:**
- `archive` (optional): Specific archive to verify. If not specified, verifies all archives.

**Options:**
- `--repository-only`: Only verify repository structure (skip archive verification)
- `--archives-only`: Only verify archives (skip chunk verification)
- `--test-restore`: Run restore test on sample files
- `--sample-files <N>`: Number of sample files for restore test (default: 10)
- `--repair`: Attempt to repair issues found (not yet implemented)
- `--json`: Output results in JSON format
- `--first <N>`: Only verify first N archives
- `--last <N>`: Only verify last N archives
- `--glob-archives <pattern>`: Only verify archives matching glob pattern

## Verification Types

### 1. Chunk Verification

Verifies that stored data chunks are not corrupted:

- Reads each chunk from disk
- Decrypts if encrypted
- Decompresses data
- Computes SHA-256 hash of content
- Compares computed hash with chunk ID

**Errors Detected:**
- Corrupted chunks (hash mismatch)
- Missing chunks (referenced in index but not on disk)
- Orphaned chunks (on disk but not in index)

### 2. Repository Verification

Comprehensive check of repository structure:

- Configuration file integrity
- Manifest integrity
- Chunk index consistency
- All chunk data integrity

### 3. Archive Verification

Verifies archive-specific integrity:

- Archive metadata is readable
- All chunks referenced by the archive exist
- File metadata is consistent

### 4. Restore Testing

Automated testing of backup restoration:

- Samples files from an archive
- Attempts to restore each file to a temporary location
- Verifies restored file size matches expected
- Optionally verifies content hash

**Configuration Options:**
- `max_samples`: Maximum number of files to test
- `min_file_size`: Minimum file size to include in sample
- `max_file_size`: Maximum file size to include (0 = no limit)
- `verify_hash`: Whether to verify content hash after restore
- `test_all`: Test all files (ignore max_samples)

## Verification Reports

### VerifyReport

Report from chunk verification:

```rust
pub struct VerifyReport {
    pub total_chunks: usize,      // Total chunks in repository
    pub verified_chunks: usize,   // Successfully verified chunks
    pub corrupted_chunks: Vec<ChunkId>,  // Corrupted chunk IDs
    pub missing_chunks: Vec<ChunkId>,    // Missing chunk IDs
    pub orphaned_chunks: Vec<ChunkId>,   // Orphaned chunk IDs
    pub duration: Duration,       // Verification duration
    pub bytes_verified: u64,      // Total bytes verified
}
```

### ArchiveVerifyReport

Report from archive verification:

```rust
pub struct ArchiveVerifyReport {
    pub archive_name: String,
    pub total_items: usize,       // Files + directories in archive
    pub verified_items: usize,    // Successfully verified items
    pub missing_chunks: Vec<(PathBuf, ChunkId)>,  // Missing chunks per file
    pub errors: Vec<String>,      // Error messages
}
```

### RestoreTestReport

Report from restore testing:

```rust
pub struct RestoreTestReport {
    pub archive_name: String,
    pub total_files: usize,       // Total files in archive
    pub files_sampled: usize,     // Files tested
    pub files_restored: usize,    // Successfully restored files
    pub bytes_restored: u64,      // Bytes restored
    pub duration: Duration,       // Test duration
    pub success: bool,            // Whether all tests passed
    pub errors: Vec<String>,      // Error messages
}
```

## Verification History

Borg-RS tracks verification results over time in `verification_history.json`:

```rust
pub struct VerificationHistory {
    pub repository_id: String,
    pub entries: Vec<VerificationHistoryEntry>,
    pub max_entries: usize,  // Default: 100
}

pub struct VerificationHistoryEntry {
    pub timestamp: DateTime<Utc>,
    pub verification_type: VerificationType,
    pub passed: bool,
    pub error_count: usize,
    pub duration_secs: f64,
    pub bytes_verified: u64,
    pub summary: String,
}
```

### Verification Types

- `FullRepository`: Complete repository verification
- `ChunksOnly`: Chunk-only verification
- `Archive`: Single archive verification
- `RestoreTest`: Restore test
- `QuickCheck`: Quick manifest/config check

## Best Practices

### Regular Verification

1. **Weekly**: Run basic verification
   ```bash
   borg verify --repo /path/to/repo
   ```

2. **Monthly**: Run verification with restore testing
   ```bash
   borg verify --repo /path/to/repo --test-restore --sample-files 50
   ```

3. **After Backup**: Quick check (automatically done by borg create)

### Handling Errors

**Corrupted Chunks:**
- Identify which archives reference the corrupted chunks
- Re-create affected archives from source data
- Consider running `borg compact` after fixing issues

**Missing Chunks:**
- Check for disk errors or incomplete writes
- If chunks are truly missing, affected files cannot be restored
- Re-create affected archives from source data

**Orphaned Chunks:**
- Not an error, just wasted space
- Run `borg compact` to reclaim space

## API Usage

### Verifying Chunks

```rust
use borg_core::repository::Repository;

let repo = Repository::open(path, Some("passphrase"))?;

// Verify single chunk
let is_valid = repo.verify_chunk(&chunk_id)?;

// Verify all chunks
let report = repo.verify_all(None)?;
if report.is_ok() {
    println!("All {} chunks verified!", report.verified_chunks);
} else {
    println!("Found {} errors", report.error_count());
}
```

### Repository-Wide Verification

```rust
use borg_core::repository::Repository;
use borg_core::verification::ConsoleProgress;

let repo = Repository::open(path, Some("passphrase"))?;

// Full repository verification with progress
let progress = ConsoleProgress;
let report = repo.verify_repository(Some(&progress))?;

println!("Verification completed in {:?}", report.total_duration);
println!("Status: {}", if report.is_ok() { "PASSED" } else { "FAILED" });
```

### Restore Testing

```rust
use borg_core::repository::Repository;
use borg_core::verification::RestoreTestConfig;

let repo = Repository::open(path, Some("passphrase"))?;

// Quick restore test
let config = RestoreTestConfig::quick();
let report = repo.test_restore("my-archive", config)?;

// Thorough restore test
let config = RestoreTestConfig::thorough();
let report = repo.test_restore("my-archive", config)?;

println!("Restore test: {}/{} files passed", 
    report.files_restored, report.files_sampled);
```

### Verification History

```rust
use borg_core::verification::{VerificationHistory, VerificationHistoryEntry};

// Load history
let mut history = VerificationHistory::load(repo_path, &repo_id)?;

// Check last verification
if let Some(last) = history.last_verification() {
    println!("Last verified: {}", last.timestamp);
    println!("Status: {}", if last.passed { "PASSED" } else { "FAILED" });
}

// Add new entry
let entry = VerificationHistory::entry_from_verify_report(&report);
history.add_entry(entry);

// Save history
history.save(repo_path)?;
```

## Error Codes

| Exit Code | Meaning |
|-----------|---------|
| 0 | Verification passed |
| 1 | Verification failed with errors |
| 2 | Command error (invalid arguments, etc.) |

## See Also

- [Repository Management](repository.md)
- [Archive Operations](archives.md)
- [Data Recovery](recovery.md)

````
