
**Borg-rs — A Rust Rewrite of BorgBackup**

- **Summary:** Borg-rs is a complete, modern rewrite of Borg Backup implemented in Rust. It preserves Borg's core design principles (deduplicated, content-addressed backups) while adding stronger safety, concurrency, and extensibility. First-class remote backends include WebDAV, S3-compatible buckets, and SFTP (usable via the `--remote-repo` option), plus local and custom storage drivers.

**Key Features**
- **Compatibility:** Implements familiar Borg concepts (archives, repositories, chunking, index/catalog) with a CLI parity goal.
- **Remote Backends:** WebDAV, S3 (compatible), SFTP, local filesystem, and pluggable remotes via the `--remote-repo` abstraction.
- **Daemon & Scheduler:** Background service `borgd` for scheduled, centralized backup management and job coordination.
- **Performance & Safety:** Rust-based implementation for memory safety and high-concurrency I/O.
- **Verification & Integrity:** Repository verification, cryptographic integrity checks, and configurable compression/encryption.

**Project Structure**
- **`borg-cli/`**: Command-line interface binary and subcommands (create, extract, list, prune, verify, etc.).
- **`borg-core/`**: Core library (chunker, storage, crypto, repository, catalog, verification logic) intended for reuse.
- **`borg-daemon/`**: Long-running service for scheduling, job state, IPC and notifications.
- **`config/`**: Example configuration and job templates. See [config/borgd.yaml.example](config/borgd.yaml.example).
- **`docs/`**: Supplemental design and operational docs (e.g., [docs/verification.md](docs/verification.md), [docs/webdav_pipeline.md](docs/webdav_pipeline.md)).
- **`systemd/`**: Example unit files for running `borg` and `borgd` as services (see [systemd/borgd.service](systemd/borgd.service)).

**Architecture & Design**
- **Overview:** See the high-level design and component interactions in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
- **Separation of Concerns:** `borg-core` implements repository, storage, chunking, and verification primitives; `borg-cli` and `borg-daemon` provide UX and orchestration layers.

**Getting Started (Developer)**
- **Build:** `cargo build --workspace --release`
- **Run CLI locally:** `target/debug/borg --help` or `cargo run -p borg-cli -- --help`
- **Run daemon:** `cargo run -p borg-daemon --release`

**Configuration & Examples**
- **Example config:** Start from [config/borgd.yaml.example](config/borgd.yaml.example).
- **Remote repo usage:** Use `--remote-repo webdav://...`, `--remote-repo s3://bucket/...`, or `--remote-repo sftp://user@host/path` when creating or accessing repositories.

**Operational Notes**
- **Verification:** Use the `verify` subcommand to run integrity checks; see [docs/verification.md](docs/verification.md) for recommended workflows.
- **WebDAV pipeline:** Implementation details and expected server behavior are in [docs/webdav_pipeline.md](docs/webdav_pipeline.md).
- **Windows build:** Platform guidance available in [docs/WINDOWS_BUILD.md](docs/WINDOWS_BUILD.md).

**Contributing & Roadmap**
- **Contributions:** We welcome issues and pull requests. Follow repository conventions and target the appropriate crate (`borg-core`, `borg-cli`, or `borg-daemon`).
- **Roadmap highlights:** Improving S3 semantics, expanding remote-driver plugin API, and polishing compatibility with existing Borg repositories.

**License**
- This project is available under the terms in the repository `LICENSE` file.

For more details, see the architecture notes and individual crate READMEs under the `borg-rs/` folder.

