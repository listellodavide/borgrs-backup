## AGENTS.md

### Purpose This document defines mandatory rules and expectations for AI agents and human contributors working on this Rust codebase.

Goals: - Correctness - Safety - Security - Performance - Maintainability - Async-first design - Cross-platform
compatibility (Linux, macOS, Windows) Deviation from this document is considered a bug unless explicitly justified. ##
Rust Version & Tooling - Minimum Rust version: stable (latest - 1) - Edition: 2024 (or newer if specified in
Cargo.toml) - Toolchain: rustup-managed - Mandatory tools: - clippy (no warnings allowed) - rustfmt (default config
unless specified) - cargo-audit for security-sensitive code - cargo-deny when dependencies are critical Agents must
never assume nightly features unless explicitly allowed. ## Async & Concurrency Rules - Async runtime: `tokio` (unless
otherwise stated) - Never block the async runtime: - ❌ std::thread::sleep - ❌ blocking I/O inside async contexts -
Use: - tokio::time - tokio::fs - tokio::net - Prefer structured concurrency - All spawned tasks must be: - awaited, OR -
clearly documented as detached background tasks No hidden global executors. ## Error Handling - No `unwrap()` or
`expect()` in production code - Errors must: - use `Result<T, E>` - propagate with `?` - preserve context (use
`thiserror` or `anyhow` where appropriate) - Panics are only allowed for: - impossible states - invariant violations -
tests Errors must be actionable and logged. ## Security Requirements - Assume all input is untrusted - Validate: -
filesystem paths - network input - environment variables - Avoid: - shell invocation - insecure temp files - unchecked
deserialization - Prefer: - constant-time comparisons for secrets - explicit permission models - least-privilege design
No hardcoded secrets. Ever. ## Dependency Policy - Minimize dependencies - Prefer well-maintained crates - No abandoned
or unmaintained crates - All dependencies must have: - a clear purpose - a documented reason Agents must not introduce
dependencies casually. ## Cross-Platform Behavior Code must work on: - Linux - macOS - Windows Rules: - No OS-specific
assumptions unless behind `cfg` - Use `std::env`, `dirs`, or platform APIs properly - Paths must use `Path` /
`PathBuf` - Line endings must be platform-safe Document platform differences explicitly. ## Time, Clock & System APIs -
Prefer monotonic clocks where possible - System time may drift — handle gracefully - Network time (NTP) is optional and
best-effort - Never fail hard due to clock issues Fallbacks must always exist. ## Logging & Observability - Use
structured logging (`tracing`) - No `println!` in production code - Log levels must be meaningful: - ERROR: action
required - WARN: unexpected but recoverable - INFO: state transitions - DEBUG/TRACE: diagnostics Logs must never leak
secrets. ## Testing - Unit tests for core logic - Integration tests for I/O and async behavior - Property-based tests
where applicable - Tests must be: - deterministic - isolated - fast Flaky tests are bugs. ## Code Style & Architecture -
Prefer small, composable functions - Avoid deep nesting - Explicit lifetimes only when needed - Public APIs must be
documented - Unsafe code: - discouraged - must be justified - must include safety comments Readable code beats clever
code. ## AI Agent Behavior Rules AI agents MUST: - Read existing code before modifying it - Follow established
patterns - Ask for clarification if intent is ambiguous - Never silently change public APIs - Never remove security
checks - Never invent undocumented behavior If unsure: stop and explain. ## Final Rule Correctness > Performance >
Convenience If a tradeoff exists, document it. This document is authoritative.