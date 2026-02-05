# Borg-RS Implementation Status & Reasoning

## Current Progress

### Completed Tasks

#### Phase 1, Task 1: Automated Integrity Verification & Restore Testing (COMPLETE)
**Files Created/Modified:**
- `borg-cli/src/commands/verify.rs` - New CLI command for comprehensive verification
- `borg-core/src/verification.rs` - Enhanced with:
  - `RepositoryIntegrityReport` - Full repository check report
  - `VerificationStatistics` - Metrics tracking
  - `RestoreTestReport` & `RestoreTestConfig` - Automated restore testing
  - `VerificationHistory` - Persistent verification history
  - `verify_repository()` method - Full integrity check
  - `test_restore()` method - Automated restore testing
- `borg-cli/src/main.rs` - Added `Verify` command and `VerifyArgs`
- `borg-cli/src/commands/mod.rs` - Added verify module
- `borg-core/src/lib.rs` - Updated prelude exports
- `docs/verification.md` - User documentation

**Key Design Decisions:**
1. Used existing `sled` database for verification history persistence
2. Sample-based restore testing to balance thoroughness vs performance
3. Progress reporting trait for flexible progress display
4. JSON output option for scripting/automation

#### Phase 1, Task 2: Metadata, Catalog & Searchable Snapshots (COMPLETE)
**Files Created/Modified:**
- `borg-core/src/catalog.rs` - New module with:
  - `Catalog` - Sled-based searchable metadata storage
  - `CatalogArchive` & `CatalogFile` - Indexed metadata types
  - `SearchQuery` & `SearchResult` - Query API
  - `ArchiveDiff` & `DiffEntry` - Archive comparison
- `borg-cli/src/commands/search.rs` - New search command
- `borg-cli/src/commands/diff.rs` - Enhanced diff implementation
- `borg-cli/src/commands/list.rs` - Enhanced with sorting/filtering
- `borg-cli/src/main.rs` - Added Search command
- `borg-cli/src/commands/mod.rs` - Added search module

**Key Design Decisions:**
1. Used sled (already a dependency) rather than SQLite to minimize dependencies
2. Implemented glob pattern matching for file search
3. Added comprehensive filtering: by size, date, archive, file type
4. Diff shows additions, deletions, and modifications with content change detection

---

## Next Tasks To Implement

### Phase 2, Task 3: Scheduling, Heartbeat & Monitoring (IN PROGRESS)

**What needs to be done:**

1. **Enhanced Scheduler Features** (`borg-core/src/scheduler.rs` - may need to create or enhance)
   - Add `last_success` timestamp tracking to schedule state
   - Implement missed backup detection (compare expected vs actual run times)
   - Create heartbeat mechanism (file or socket-based)

2. **Notification System** (new module: `borg-core/src/notifications.rs`)
   - Design plugin architecture with `Notifier` trait
   - Implement concrete notifiers:
     - `EmailNotifier` - SMTP-based email alerts
     - `WebhookNotifier` - HTTP POST to configurable endpoints
     - `SlackNotifier` - Slack incoming webhooks
   - Configuration structure for notification preferences

3. **Alert Rules** (part of scheduler/notifications)
   - Alert threshold configuration (e.g., backup older than X hours)
   - Failure detection (consecutive failures, specific error types)
   - Retry logic with exponential backoff

4. **CLI Commands**
   - `borgrs notify test` - Test notification configuration
   - `borgrs status` - Show backup health/heartbeat status

**Files to create/modify:**
- `borg-core/src/notifications.rs` (new)
- `borg-core/src/scheduler.rs` (enhance existing or create)
- `borg-cli/src/commands/notify.rs` (new)
- `borg-cli/src/commands/status.rs` (new or enhance)

**Dependencies to consider:**
- `lettre` for email (check Cargo.toml)
- `reqwest` for webhooks (already available)

---

### Phase 2, Task 4: Retention Policy & Cleanup Automation

**What needs to be done:**

1. **Retention Engine** (`borg-core/src/retention.rs`)
   - `RetentionPolicy` struct with rules:
     - Keep last N backups
     - Keep daily/weekly/monthly/yearly
     - Size-based limits
   - `RetentionEvaluator` to decide which archives to keep/prune

2. **Prune Command Enhancement** (`borg-cli/src/commands/prune.rs`)
   - Currently minimal - needs full implementation
   - Dry-run mode to preview what would be deleted
   - Integration with retention policies
   - Safety checks before deletion

3. **Automated Cleanup**
   - Integrate retention evaluation into scheduler
   - Post-backup automatic pruning option
   - Cleanup reporting

---

### Phase 3, Task 5: Immutable & Tamper-Resistance Features

**What needs to be done:**

1. **Append-only Repository Mode**
   - Repository mode flag in config
   - Prevent deletion operations in append-only mode
   - Allow only additive operations

2. **Retention Enforcement**
   - Lock mechanism for archives during retention period
   - Prevent manual deletion during locked period

3. **Cryptographic Signing**
   - Sign archive manifests
   - Verify signatures on archive load
   - Tamper detection

---

### Phase 3, Task 6: Security Features (Encryption & Key Management)

**What needs to be done:**

1. **Key Rotation**
   - Mechanism to re-encrypt with new key
   - Key versioning

2. **External Key Providers**
   - KMS integration trait
   - HashiCorp Vault support

---

## Architecture Notes

### Current Module Structure
```
borg-core/src/
├── lib.rs           # Public exports
├── chunker.rs       # Content-defined chunking
├── compression.rs   # LZ4/Zstd compression
├── crypto.rs        # AES-256-GCM encryption
├── repository.rs    # Repository operations
├── archive.rs       # Archive creation/extraction
├── verification.rs  # Integrity checking (enhanced)
├── catalog.rs       # Searchable metadata (new)
├── cache.rs         # Sled-based caching
├── exclusion.rs     # File exclusion patterns
├── remote.rs        # Remote repository access
└── error.rs         # Error types
```

### CLI Command Structure
```
borg-cli/src/commands/
├── mod.rs
├── init.rs          # Repository initialization
├── create.rs        # Backup creation
├── extract.rs       # File restoration
├── list.rs          # Archive listing (enhanced)
├── info.rs          # Repository info
├── delete.rs        # Archive deletion
├── prune.rs         # Archive pruning (needs enhancement)
├── check.rs         # Legacy verification
├── verify.rs        # New verification (new)
├── search.rs        # File search (new)
├── diff.rs          # Archive comparison (enhanced)
├── mount.rs         # FUSE mounting
├── umount.rs        # FUSE unmounting
├── rename.rs        # Archive renaming
├── compact.rs       # Repository compaction
├── key.rs           # Key management
├── export.rs        # Archive export
├── import.rs        # Archive import
├── config.rs        # Configuration
└── benchmark.rs     # Performance testing
```

### Key Patterns Used

1. **Error Handling**: Uses `BorgError` enum with `Result<T>` alias
2. **Progress Reporting**: `ProgressReporter` trait for flexible progress display
3. **Serialization**: Serde for JSON/MessagePack serialization
4. **Storage**: Sled embedded database for metadata caching
5. **CLI**: Clap derive macros for argument parsing
6. **Async**: Tokio runtime for async operations

### Dependencies Available
- `sled` - Embedded database
- `serde`, `serde_json` - Serialization
- `chrono` - Date/time handling
- `sha2` - SHA-256 hashing
- `aes-gcm` - Encryption
- `lz4_flex`, `zstd` - Compression
- `clap` - CLI parsing
- `tokio` - Async runtime
- `tracing` - Logging
- `glob` - Pattern matching
- `tempfile` - Temporary files

---

## Implementation Guidelines for Next Developer

1. **Before implementing a new feature:**
   - Read existing similar modules for patterns
   - Check `Cargo.toml` for available dependencies
   - Review `lib.rs` prelude for what's already exported

2. **When adding CLI commands:**
   - Add to `commands/mod.rs`
   - Add Args struct in `main.rs` or in the command file
   - Add to Commands enum in `main.rs`
   - Add dispatch in `main.rs` match statement

3. **When adding core functionality:**
   - Add module to `borg-core/src/lib.rs`
   - Export important types in prelude
   - Use existing error types or extend `BorgError`

4. **Testing considerations:**
   - Unit tests in same file with `#[cfg(test)]` module
   - Integration tests in `tests/` directory
   - Use `tempfile` for test directories

5. **Documentation:**
   - Doc comments on public types and functions
   - User documentation in `docs/` directory
   - Update `task.md` when completing items
