# Building on Windows

This project includes a Linux daemon (`borg-daemon`) that uses Unix-specific features like systemd integration, Unix sockets, and POSIX signals. These components are not available on Windows.

## Building the CLI Tool

To build the command-line tool (`borg`) on Windows, use:

```powershell
cargo build --release --workspace
```

This will build:
- `borg-core` - Core backup library
- `borg-cli` - Command-line interface (`borg.exe`)

The daemon (`borg-daemon`) is automatically excluded from the workspace on Windows.

## What Works on Windows

- ✅ **borg-cli**: Full command-line backup tool
  - Create, list, extract, delete archives
  - Repository management
  - Encryption and compression
  - WebDAV and SSH remote repositories
  - All backup operations

## What Doesn't Work on Windows

- ❌ **borg-daemon**: Linux-only scheduled backup daemon
  - Requires systemd integration (`sd-notify`)
  - Requires Unix domain sockets
  - Requires POSIX signal handling
  - Requires journald logging

## Alternative for Scheduled Backups on Windows

Instead of using `borg-daemon`, you can schedule backups on Windows using:

1. **Windows Task Scheduler**: Schedule `borg.exe` commands
2. **PowerShell Scripts**: Create scripts that call `borg.exe`
3. **Third-party schedulers**: Use tools like cron-like schedulers for Windows

Example PowerShell script for scheduled backup:

```powershell
# backup.ps1
$env:BORG_REPO = "C:\Backups\MyRepo"
$env:BORG_PASSPHRASE = "your-secure-passphrase"

& "C:\path\to\borg.exe" create ::"{hostname}-{now}" C:\DataToBackup
```

Then schedule this script using Windows Task Scheduler.

## WebDAV Usage Example

Borg-RS supports backing up directly to WebDAV servers (Nextcloud, Apache, rclone, etc.).

### 1. Initialize Repository

```powershell
.\borg.exe init --webdav-url http://192.168.188.77/borg/ --webdav-user borg --webdav-pass hello --encryption repokey
```

### 2. Create Backup

You can pass credentials explicitly to the create command. Compression is enabled by default (zstd).

```powershell
.\borg.exe create --repo http://192.168.188.77/borg/ --webdav-user borg --webdav-pass hello backup-2024 C:\Users\dukov82\Documents\
```

Alternatively, you can embed credentials in the URL (less secure in command history):
`--repo http://borg:hello@192.168.188.77/borg/`

## Build Requirements

- Rust toolchain (MSVC or GNU)
- Visual Studio Build Tools (for MSVC toolchain)
- OpenSSL (if using features that require it)

## Troubleshooting

If you encounter build errors related to `sd-notify` or Unix-specific dependencies, ensure you're using the workspace build command which automatically excludes the daemon:

```powershell
cargo build --release --workspace
```

Do NOT try to build the daemon directly on Windows:
```powershell
# This will fail on Windows:
cargo build --release -p borg-daemon
```
