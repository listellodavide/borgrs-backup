# new_task.md ## 
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
