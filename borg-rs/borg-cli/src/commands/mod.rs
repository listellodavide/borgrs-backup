//! CLI command implementations

pub mod init;
pub mod create;
pub mod restore;
pub mod list;
pub mod info;
pub mod delete;
pub mod prune;
pub mod check;
pub mod verify;
pub mod search;
pub mod mount;
pub mod umount;
pub mod diff;
pub mod rename;
pub mod compact;
pub mod key;
pub mod export;
pub mod import;
pub mod config;
pub mod benchmark;

use anyhow::{Context, Result};
use borg_core::repository::Repository;
use borg_core::storage::{parse_storage_config, build_operator, StorageConfig};

/// Get repository path from CLI args or environment
pub fn get_repo_path(cli: &crate::Cli) -> Result<String> {
    if let Some(repo) = &cli.local_repo {
        return Ok(repo.clone());
    }
    if let Some(repo) = &cli.remote_repo {
        return Ok(repo.clone());
    }
    cli.repo.clone()
        .or_else(|| std::env::var("BORG_REPO").ok())
        .context("Repository not specified. Use --repo, --local-repo, --remote-repo or set BORG_REPO environment variable")
}

/// Get passphrase from environment or prompt
pub async fn get_passphrase(prompt: &str) -> Result<String> {
    // Check environment first
    if let Ok(pass) = std::env::var("BORG_PASSPHRASE") {
        return Ok(pass);
    }

    // Check passphrase command
    if let Ok(cmd) = std::env::var("BORG_PASSCOMMAND") {
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .output()
            .await
            .context("Failed to execute BORG_PASSCOMMAND")?;

        if output.status.success() {
            return Ok(String::from_utf8(output.stdout)?.trim().to_string());
        }
    }

    // Check passphrase file
    if let Ok(_file) = std::env::var("BORG_PASSPHRASE_FD") {
        // Read from file descriptor
        // This is a simplified implementation
        anyhow::bail!("BORG_PASSPHRASE_FD not supported in this version");
    }

    // Prompt user
    let pass = rpassword::prompt_password(prompt)
        .context("Failed to read passphrase")?;

    Ok(pass)
}

/// Open repository with passphrase handling
pub async fn open_repository(repo_str: &str) -> Result<Repository> {
    let mut config = parse_storage_config(repo_str)?;

    // Inject S3 credentials from env vars if they were set by main.rs
    // This ensures that even if the URL doesn't have credentials, we use the ones from CLI/env
    if let StorageConfig::S3 { access_key, secret_key, session_token, .. } = &mut config {
        if access_key.is_none() {
            if let Ok(ak) = std::env::var("S3_ACCESS_KEY") {
                *access_key = Some(ak);
            }
        }
        if secret_key.is_none() {
            if let Ok(sk) = std::env::var("S3_SECRET_KEY") {
                *secret_key = Some(sk);
            }
        }
        if session_token.is_none() {
            if let Ok(tok) = std::env::var("S3_SESSION_TOKEN") {
                *session_token = Some(tok);
            }
        }
    }

    let op = build_operator(config)?;
    
    // Try opening without passphrase first (for unencrypted repos)
    match Repository::open(op.clone(), repo_str.to_string(), None).await {
        Ok(repo) => Ok(repo),
        Err(e) => {
            // Check if it's a passphrase error
            if matches!(e, borg_core::error::BorgError::InvalidPassphrase) {
                // Get passphrase and retry
                let passphrase = get_passphrase("Enter passphrase: ").await?;
                Repository::open(op, repo_str.to_string(), Some(&passphrase)).await
                    .map_err(|e| anyhow::anyhow!("Failed to open repository: {}", e))
            } else {
                Err(anyhow::anyhow!("Failed to open repository: {}", e))
            }
        }
    }
}

/// Format size in human-readable form
pub fn format_size(bytes: u64) -> String {
    humansize::format_size(bytes, humansize::BINARY)
}

/// Format duration in human-readable form  
pub fn format_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{:.1}s", secs)
    } else if secs < 3600.0 {
        format!("{:.1}m", secs / 60.0)
    } else {
        format!("{:.1}h", secs / 3600.0)
    }
}
