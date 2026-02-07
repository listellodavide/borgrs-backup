```markdown
# Borgrs-Backup Feature Implementation Tasks

## Phase 1: Safety & Integrity

### 1. Automated Integrity Verification & Restore Testing
- [X] Core verification module
  - [X] Add chunk checksum verification to `repository.rs`
  - [X] Implement `verify` subcommand in CLI
  - [X] Add repository-wide integrity check function
  - [X] Create verification report structure
- [X] Testing infrastructure
  - [X] Implement automated restore test command
  - [X] Add sample file extraction and validation
  - [X] Create verification statistics tracking
- [X] Documentation
  - [X] Write user guide for verification commands
  - [X] Document verification workflow
  - [ ] Add examples to README

### 2. Metadata, Catalog & Searchable Snapshots
- [X] Snapshot metadata enhancement
  - [X] Create comprehensive snapshot database schema
  - [X] Add snapshot metadata to `archive.rs`
  - [X] Implement change tracking between snapshots
- [X] Search and query features
  - [X] Implement `snapshots` list command with filtering
  - [X] Add `search` command for file lookup across snapshots
  - [X] Create `diff` command implementation (currently stubbed)
- [X] Metadata storage
  - [X] Add sled-based catalog for efficient queries
  - [X] Index all backup metadata efficiently
  - [X] Track file modifications, sizes, dates
- [ ] Documentation
  - [ ] User guide for search features
  - [ ] Examples of common queries

## Phase 2: Monitoring & Alerts

### 3. Scheduling, Heartbeat & Monitoring
- [ ] Enhanced scheduler features
  - [ ] Add last-success timestamp tracking
  - [ ] Implement missed backup detection
  - [ ] Create heartbeat file/socket mechanism
- [ ] Notification system
  - [ ] Design notification plugin architecture
  - [ ] Implement email notifications
  - [ ] Add webhook support
  - [ ] Add Slack integration
- [ ] Alert rules
  - [ ] Configure alert thresholds
  - [ ] Add failure detection logic
  - [ ] Implement retry with exponential backoff
- [ ] Testing
  - [ ] Unit tests for notification system
  - [ ] Integration tests for scheduler alerts
- [ ] Documentation
  - [ ] Configuration guide for notifications
  - [ ] Alert setup examples

### 4. Retention Policy & Cleanup Automation
- [ ] Retention engine
  - [ ] Design retention policy structure
  - [ ] Implement time-based retention (daily/weekly/monthly)
  - [ ] Add size-based pruning
  - [ ] Create retention policy validation
- [ ] Prune command implementation
  - [ ] Complete `prune` command (currently minimal)
  - [ ] Add dry-run mode
  - [ ] Implement safe delete with verification
- [ ] Automated cleanup
  - [ ] Integrate retention into daemon scheduler
  - [ ] Add automatic pruning jobs
  - [ ] Create cleanup reports
- [ ] Testing
  - [ ] Unit tests for retention policies
  - [ ] Integration tests for pruning
- [ ] Documentation
  - [ ] Retention policy guide
  - [ ] Best practices for backup retention

## Phase 3: Security

### 5. Immutable & Tamper-Resistance Features
- [ ] Append-only repository mode
  - [ ] Design append-only repository format
  - [ ] Prevent snapshot deletion in append-only mode
  - [ ] Add repository mode flags
- [ ] Retention enforcement
  - [ ] Implement mandatory retention policies
  - [ ] Prevent manual deletion during retention period
  - [ ] Add lock mechanisms for snapshot protection
- [ ] Cryptographic signing
  - [ ] Sign snapshot metadata
  - [ ] Verify signatures on read
  - [ ] Detect tampering attempts
- [ ] WORM backend support
  - [ ] Design object-lock compatible backends
  - [ ] Add immutability flags to cloud backends
- [ ] Testing
  - [ ] Security tests for tamper detection
  - [ ] Append-only mode verification
- [ ] Documentation
  - [ ] Security model documentation
  - [ ] Ransomware protection guide

### 6. Security Features (Encryption & Key Management)
- [ ] Enhanced encryption
  - [ ] Review existing AES-256-GCM implementation
  - [ ] Add encryption verification
  - [ ] Ensure AEAD mode is used properly
- [ ] Key management improvements
  - [ ] Implement key rotation
  - [ ] Add external key provider support (KMS)
  - [ ] Add HashiCorp Vault integration
  - [ ] Improve key storage separation
- [ ] Security hardening
  - [ ] Add encrypted metadata support
  - [ ] Implement secure key deletion
  - [ ] Add key backup/recovery mechanism
- [ ] Testing
  - [ ] Security audit of crypto implementation
  - [ ] Key management tests
- [ ] Documentation
  - [ ] Encryption guide
  - [ ] Key management best practices

## Phase 4: Usability & Backends

### 7. Plugin-Style Backends (Local, SFTP, Cloud)
- [ ] Backend architecture
  - [ ] Design `BackupBackend` trait
  - [ ] Refactor existing repository to use trait
  - [ ] Add backend registry/discovery
- [ ] Local filesystem backend
  - [ ] Extract current implementation to trait impl
  - [ ] Add optimizations for local storage
- [ ] SFTP/SSH backend
  - [ ] Implement SSH backend using russh
  - [ ] Add connection pooling
  - [ ] Handle authentication (keys, passwords)
- [ ] Cloud backends
  - [ ] Amazon S3 backend
  - [ ] Backblaze B2 backend
  - [ ] Azure Blob Storage backend
  - [ ] Google Cloud Storage backend
- [ ] Backend features
  - [ ] Connection retry logic
  - [ ] Bandwidth throttling
  - [ ] Resume support for interrupted uploads
- [ ] Testing
  - [ ] Unit tests for each backend
  - [ ] Integration tests with cloud services
  - [ ] Mock backend for testing
- [ ] Documentation
  - [ ] Backend configuration guide
  - [ ] Examples for each backend type

### 8. User Experience & CLI Ergonomics
- [ ] CLI improvements
  - [ ] Add shell completion (bash, zsh, fish)
  - [ ] Improve colored output
  - [ ] Better error messages with suggestions
  - [ ] Add progress bars for all operations
- [ ] Interactive features
  - [ ] Interactive restore with file browser
  - [ ] Interactive snapshot selection
  - [ ] Confirmation prompts for destructive operations
- [ ] Output formatting
  - [ ] JSON output mode for scripting
  - [ ] Table formatting for lists
  - [ ] Summary statistics
- [ ] Testing
  - [ ] CLI integration tests
  - [ ] User experience testing
- [ ] Documentation
  - [ ] Updated CLI reference
  - [ ] Quick start guide
  - [ ] Tutorial for new users

### 9. Community & Contribution Strategy
- [ ] Documentation
  - [ ] Comprehensive README with diagrams
  - [ ] Feature comparison table
  - [ ] Installation guide for multiple platforms
  - [ ] Usage examples
- [ ] Contribution infrastructure
  - [ ] CONTRIBUTING.md guide
  - [ ] Code of conduct
  - [ ] Issue templates (bug, feature request)
  - [ ] Pull request template
- [ ] Developer documentation
  - [ ] Architecture documentation updates
  - [ ] API documentation
  - [ ] Development environment setup
- [ ] Community building
  - [ ] Create GitHub Discussions
  - [ ] Add badges (build status, coverage, etc.)
  - [ ] Setup CI/CD pipelines

### 10. Testing, Benchmarking, and CI
- [ ] Test coverage expansion
  - [ ] Unit tests for all core modules
  - [ ] Integration tests for CLI commands
  - [ ] End-to-end backup/restore tests
  - [ ] Target 80%+ code coverage
- [ ] Benchmarking suite
  - [ ] Expand existing benchmark command
  - [ ] Add chunking performance tests
  - [ ] Add compression benchmarks
  - [ ] Add large directory tests
  - [ ] Track performance over time
- [ ] CI/CD pipeline
  - [ ] GitHub Actions workflow
  - [ ] Automated testing on PR
  - [ ] Multi-platform testing (Linux, macOS, Windows)
  - [ ] Automated release builds
- [ ] Quality gates
  - [ ] Linting with clippy
  - [ ] Format checking with rustfmt
  - [ ] Security audit with cargo-audit
  - [ ] Dependency updates automation
- [ ] Documentation
  - [ ] Testing guide for contributors
  - [ ] Benchmark interpretation guide

# Phase 5: General Refactory
Context & Goal

This task set builds the core ingestion and storage pipeline for a borg-style backup system written in Rust, where the backup source is a remote WebDAV server. The goal is to reliably stream large, potentially unreliable remote files, split them into content-defined chunks, deduplicate aggressively, and store them in an append-only, verifiable repository. Each step is intentionally isolated to keep changes small, testable, and composable, while converging toward a production-grade backup engine with deterministic behavior, crash safety, and long-term format stability. A senior Rust developer should focus on correctness, streaming semantics, bounded memory usage, and clear APIs, with the understanding that these components form the foundation for encryption, retention, restore, and verification features later in the system.

11. WebDAV Source Scanner & Metadata Cache -
[ ] WebDAV discovery layer -
[ ] Implement recursive PROPFIND walker -
[ ] Parse WebDAV XML responses into typed structs -
[ ] Normalize paths and handle trailing slashes -
[ ] Metadata cache -
[ ] Create `WebDavFileMeta` struct (path, size, mtime, etag) -
[ ] Store metadata cache locally (sled/sqlite) -
[ ] Detect unchanged files using size + mtime -
[ ] Incremental scan support -
[ ] Skip unchanged files during scan -
[ ] Mark deleted files -
[ ] Testing -
[ ] Mock WebDAV server responses -
[ ] Unit tests for metadata diff logic -
[ ] Documentation -
[ ] WebDAV scan behavior explanation

## 12. Streaming WebDAV Reader with Range Support -
[ ] Streaming reader abstraction -
[ ] Implement `WebDavReader` using reqwest async streams -
[ ] Add configurable read buffer size -
[ ] Range request support -
[ ] Implement HTTP Range header handling -
[ ] Detect and handle servers without Range support -
[ ] Resume logic -
[ ] Add offset-based resume support -
[ ] Validate resumed data integrity -
[ ] Concurrency control -
[ ] Limit parallel readers -
[ ] Add timeout and retry logic -
[ ] Testing -
[ ] Unit tests for range math and offsets -
[ ] Integration tests with partial downloads -
[ ] Documentation -
[ ] Streaming, resume, and retry behavior

## 13. Content-Defined Chunking Pipeline -
[ ] Chunker integration -
[ ] Integrate FastCDC into streaming pipeline -
[ ] Make min/avg/max chunk sizes configurable -
[ ] Chunk metadata -
[ ] Create `ChunkDescriptor` (hash placeholder, size, offsets) -
[ ] Track chunk boundaries per file -
[ ] Pipeline backpressure -
[ ] Ensure bounded memory usage -
[ ] Prevent unbounded buffering -
[ ] Testing -
[ ] Deterministic chunking tests -
[ ] Shifted-content regression tests -
[ ] Documentation -
[ ] CDC design and dedup rationale

## 14. Chunk Hashing & Dedup Index Layer -
[ ] Hashing -
[ ] Add streaming BLAKE3 hashing -
[ ] Ensure hash stability across runs -
[ ] Dedup index abstraction -
[ ] Define `ChunkIndex` trait -
[ ] Implement sled-based index backend -
[ ] Deduplication logic -
[ ] Detect existing chunks before write -
[ ] Track chunk reference counts -
[ ] Index persistence -
[ ] Ensure crash-safe updates -
[ ] Implement index recovery on startup -
[ ] Testing -
[ ] Dedup correctness tests -
[ ] Index corruption and recovery tests -
[ ] Documentation -
[ ] Dedup index architecture overview

## 15. Pack File Writer (Append-Only Storage) -
[ ] Pack format definition -
[ ] Define pack header structure -
[ ] Define per-chunk entry layout -
[ ] Add pack format versioning -
[ ] Pack writer implementation -
[ ] Implement append-only writes -
[ ] Add size-based pack rotation -
[ ] Index integration -
[ ] Store pack ID, offset, and length per chunk -
[ ] Validate index ↔ pack consistency -
[ ] Integrity checks -
[ ] Detect truncated or corrupted packs -
[ ] Testing -
[ ] Pack write/read roundtrip tests -
[ ] Corruption detection tests -
[ ] Documentation -
[ ] Pack file layout and lifecycle

## 16. Compression & Encryption Pipeline -
[ ] Compression -
[ ] Integrate zstd level 9 compression -
[ ] Make compression level configurable -
[ ] Encryption -
[ ] Implement AEAD encryption (XChaCha20-Poly1305) -
[ ] Generate and store per-chunk nonces safely -
[ ] Pipeline enforcement -
[ ] Enforce compress → encrypt order - 
[ ] Add runtime validation checks - 
[ ] Key handling - 
[ ] Load repository encryption keys - 
[ ] Prevent key reuse errors - 
[ ] Testing - 
[ ] Compression/encryption roundtrip tests - 
[ ] Tampered data detection tests - 
[ ] Documentation - 
[ ] Compression and encryption pipeline explanation 

## 17. Archive File Tree Builder & Snapshot Persistence - 
[ ] Archive data model - 
[ ] Define `ArchiveNode` (file, dir, symlink) -
[ ] Store permissions, ownership, timestamps - 
[ ] Chunk mapping - 
[ ] Map files to ordered chunk lists - 
[ ] Support sparse files and large files - 
[ ] Archive persistence - 
[ ] Serialize archive tree to repository - 
[ ] Add archive format versioning - 
[ ] Restore preparation - 
[ ] Validate archive references against index - 
[ ] Testing - 
[ ] Archive reconstruction tests - 
[ ] End-to-end restore correctness tests - 
[ ] Documentation - 
[ ] Archive and snapshot data model

## 18. WebDAV Refactor & Protocol Selection -
[ ] Protocol abstraction
[ ] Extract current SSH-based transport into a `RemoteTransport` trait
[ ] Define shared request/response types for upload/download/list operations
[ ] Add transport error mapping and retry semantics
[ ] WebDAV transport implementation
[ ] Implement WebDAV client using reqwest
[ ] Support WebDAV upload (PUT) and download (GET) with streaming
[ ] Add WebDAV directory creation and listing utilities
[ ] Configuration updates
[ ] Add protocol selector parameter (ssh/webdav) in config and CLI
[ ] Validate required WebDAV settings (url, credentials, path)
[ ] Ensure backward compatibility for existing SSH configs
[ ] Integration points
[ ] Wire protocol selector into daemon scheduling and job creation
[ ] Ensure repository write path supports WebDAV backend
[ ] Documentation
[ ] Update config examples with WebDAV option
[ ] Document protocol selection behavior

11. WebDAV Source Scanner & Metadata Cache - 
[ ] WebDAV discovery layer - 
[ ] Implement recursive PROPFIND walker - 
[ ] Parse WebDAV XML responses into typed structs - 
[ ] Normalize paths and handle trailing slashes - 
[ ] Metadata cache - 
[ ] Create `WebDavFileMeta` struct (path, size, mtime, etag) - 
[ ] Store metadata cache locally (sled/sqlite) - 
[ ] Detect unchanged files using size + mtime - 
[ ] Incremental scan support - 
[ ] Skip unchanged files during scan - 
[ ] Mark deleted files - 
[ ] Testing - 
[ ] Mock WebDAV server responses - 
[ ] Unit tests for metadata diff logic - 
[ ] Documentation - 
[ ] WebDAV scan behavior explanation 

## 12. Streaming WebDAV Reader with Range Support - 
[ ] Streaming reader abstraction - 
[ ] Implement `WebDavReader` using reqwest streams - 
[ ] Add configurable read buffer size - 
[ ] Range request support - 
[ ] Implement byte-range reads - 
[ ] Handle servers without Range support gracefully - 
[ ] Resume logic - [ ] Add offset-based resume for interrupted reads - 
[ ] Verify resumed data integrity - 
[ ] Testing - 
[ ] Unit tests for range math - 
[ ] Integration test with partial downloads - 
[ ] Documentation - 
[ ] Streaming and resume behavior 

## 13. Content-Defined Chunking Pipeline - 
[ ] Chunker integration - 
[ ] Integrate FastCDC into streaming pipeline - 
[ ] Support configurable min/avg/max chunk sizes - 
[ ] Chunk metadata - 
[ ] Create `ChunkDescriptor` (hash, size, offset) - 
[ ] Track chunk boundaries per file - 
[ ] Backpressure handling - 
[ ] Ensure bounded memory usage during streaming - 
[ ] Testing - 
[ ] Deterministic chunking tests - 
[ ] Regression tests for shifted content - 
[ ] Documentation - 
[ ] Explanation of CDC and dedup benefits 

## 14. Chunk Hashing & Dedup Index Layer - 
[ ] Hashing - 
[ ] Add BLAKE3 streaming hasher - 
[ ] Validate hash consistency across runs - 
[ ] Dedup index API - 
[ ] Define `ChunkIndex` trait - 
[ ] Implement sled-based index backend - 
[ ] Dedup logic - 
[ ] Skip writing chunks already present - 
[ ] Track reference counts per chunk - 
[ ] Testing - 
[ ] Dedup correctness tests - 
[ ] Index crash-recovery tests - 
[ ] Documentation - 
[ ] Deduplication model overview 

## 15. Pack File Writer (Append-Only) - 
[ ] Pack format - 
[ ] Define pack header and chunk entry format - 
[ ] Add versioning to pack files - 
[ ] Pack writer - 
[ ] Implement append-only pack writer - 
[ ] Add size-based pack rotation - 
[ ] Index integration - 
[ ] Record pack offset and length in chunk index - 
[ ] Testing - 
[ ] Pack write/read roundtrip tests - 
[ ] Corrupted pack detection tests - 
[ ] Documentation - 
[ ] Pack file layout description 

## 16. Compression & Encryption Pipeline - 
[ ] Compression - 
[ ] Integrate zstd compression - 
[ ] Make compression level configurable - 
[ ] Encryption - 
[ ] Add AEAD encryption (XChaCha20-Poly1305) - 
[ ] Generate per-chunk nonces safely - 
[ ] Pipeline ordering - 
[ ] Enforce compress → encrypt order - 
[ ] Add pipeline validation checks - 
[ ] Testing - 
[ ] Encrypt/decrypt roundtrip tests - 
[ ] Tampered data detection tests - 
[ ] Documentation - 
[ ] Crypto pipeline explanation 

## 17. Archive File Tree Builder - 
[ ] File tree model - 
[ ] Define `ArchiveNode` (file/dir/symlink) - 
[ ] Store permissions, ownership, timestamps - 
[ ] Chunk mapping - 
[ ] Map files to ordered chunk lists - 
[ ] Store sparse file support metadata - 
[ ] Archive persistence - 
[ ] Serialize archive tree to repo - 
[ ] Version archive format - 
[ ] Testing - 
[ ] Archive reconstruction tests - 
[ ] Restore correctness tests - 
[ ] Documentation - 
[ ] Archive and snapshot data model