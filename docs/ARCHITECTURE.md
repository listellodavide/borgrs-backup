````markdown
# Borg-Rust Architecture

## Overview

Borg-Rust is a modern, Rust-based implementation inspired by BorgBackup. It provides deduplicating, compressed, and encrypted backups with a focus on safety, performance, and memory efficiency.

## Project Structure

```
borg-rust/
├── Cargo.toml              # Workspace manifest
├── borg-core/              # Core library
│   ├── src/
│   │   ├── lib.rs          # Library entry point
│   │   ├── error.rs        # Error types
│   │   ├── chunker.rs      # Content-defined chunking (Buzhash, etc.)
│   │   ├── crypto.rs       # Encryption (ChaCha20-Poly1305, AES-256-GCM)
│   │   ├── compression.rs  # Compression (zstd, lz4, zlib, lzma)
│   │   ├── exclusion.rs    # File exclusion patterns
│   │   ├── repository.rs   # Repository management
│   │   ├── archive.rs      # Archive creation/extraction
│   │   ├── cache.rs        # Chunk cache management
│   │   └── remote.rs       # Remote repository protocol
├── borg-daemon/            # Linux daemon
│   ├── src/
│   │   ├── main.rs         # Daemon entry point
│   │   ├── config.rs       # Configuration parsing
│   │   ├── scheduler.rs    # Cron-based job scheduler
│   │   ├── ipc.rs          # Unix socket control interface
│   │   ├── job.rs          # Backup job execution
│   │   └── health.rs       # Health monitoring
├── borg-cli/               # Command-line interface
│   ├── src/
│   │   ├── main.rs         # CLI entry point
│   │   ├── commands/       # Subcommand implementations
│   │   ├── output.rs       # Output formatting
│   │   └── progress.rs     # Progress display
├── systemd/                # Systemd service files
└── config/                 # Example configurations
```

## Core Components

### 1. Chunking (borg-core/chunker.rs)

Content-defined chunking using the Buzhash rolling hash algorithm:

- **Buzhash**: Variable-size chunks with configurable min/max sizes
- **Fixed**: Fixed-size chunks for comparison/testing
- **FastCDC**: Alternative CDC algorithm (optional)

The chunker produces chunks that are stable across file modifications, enabling efficient deduplication.

### 2. Deduplication

Chunks are identified by their cryptographic hash (BLAKE2b or SHA-256). The repository maintains a chunk index that maps hashes to storage locations.

```
File → Chunks → Hash → Repository Index → Stored Data
```

### 3. Compression (borg-core/compression.rs)

Multiple compression algorithms:

- **None**: No compression
- **LZ4**: Fast compression, moderate ratio
- **Zstd**: Balanced speed/ratio (recommended)
- **Zlib**: Good compatibility
- **LZMA**: Maximum compression, slower

Compression is applied per-chunk before encryption.

### 4. Encryption (borg-core/crypto.rs)

Authenticated encryption modes:

- **None**: No encryption (not recommended)
- **Repokey**: Key stored in repository, protected by passphrase
- **Keyfile**: Key stored separately from repository
- **Blake2 variants**: Use BLAKE2b for HMAC

Encryption uses ChaCha20-Poly1305 (modern) or AES-256-GCM (hardware acceleration).

### 5. Repository Structure

```
repository/
├── config              # Repository configuration (JSON/MessagePack)
├── data/               # Chunk data files
│   ├── 0/              # Segment files by hash prefix
│   ├── 1/
│   └── ...
├── index/              # Chunk index (hash → location)
├── hints/              # Optimization hints
└── lock                # Repository lock
```

### 6. Exclusion System (borg-core/exclusion.rs)

Flexible pattern matching:

- Shell-style globs (`*.log`, `cache/*`)
- Regular expressions (`re:.*\.pyc$`)
- Path prefix matching (`pp:/var/cache`)
- CACHEDIR.TAG support
- Exclude-if-present patterns

## Daemon Architecture

### Scheduler

Cron-based job scheduling using a priority queue:

```rust
struct ScheduledJob {
    name: String,
    next_run: DateTime<Utc>,
    schedule: Schedule,
    priority: u32,
}
```

Jobs are ordered by next run time, with priority as tiebreaker.

### IPC Control

Unix domain socket for daemon control:

- Status queries
- Job management (run, pause, resume)
- Configuration reload
- Graceful shutdown

Protocol: Line-based text commands with JSON responses.

### Systemd Integration

- `Type=notify`: Proper startup notification
- Watchdog: Health monitoring
- Credentials: Secure passphrase handling
- Security hardening: Namespaces, capabilities, syscall filtering

## Data Flow

### Backup Creation

```
1. Scan paths (respecting exclusions)
2. For each file:
   a. Read file contents
   b. Chunk using Buzhash
   c. For each chunk:
      - Hash chunk
      - Check if already in repository
      - If new: compress → encrypt → store
      - Add to archive manifest
3. Create archive metadata
4. Update repository index
```

### Restore

```
1. Read archive manifest
2. For each file in manifest:
   a. Look up chunks in index
   b. Read chunks from repository
   c. Decrypt → decompress
   d. Reassemble file
   e. Restore metadata (permissions, timestamps, etc.)
```

## Performance Considerations

### Memory Efficiency

- Streaming chunk processing
- Memory-mapped I/O for large files
- Bounded buffer sizes
- Chunk cache with LRU eviction

### CPU Utilization

- Parallel chunk processing
- SIMD-accelerated hashing (BLAKE2b)
- Hardware AES acceleration when available
- Async I/O for remote repositories

### I/O Optimization

- Sequential reads for backup
- Batch writes to repository
- Index caching
- Checkpoint support for interruption recovery

## Security Model

### Threat Model

- Repository server is untrusted
- Encryption key never leaves client
- Authenticated encryption prevents tampering
- No plaintext metadata in encrypted mode

### Key Derivation

```
Passphrase → Argon2id → Master Key → Encryption Key + MAC Key
```

### Integrity

- Each chunk authenticated with Poly1305/GCM
- Archive manifests are signed
- Repository index protected by MAC

## Compatibility Notes

Borg-Rust aims for format compatibility with BorgBackup where practical:

- Repository format: Compatible
- Chunk format: Compatible
- Archive metadata: Mostly compatible
- Remote protocol: Compatible (rpc)

Some advanced features may use extended formats that are Borg-Rust specific.

````
