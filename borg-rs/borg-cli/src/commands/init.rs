//! Repository initialization command

use std::path::PathBuf;
use anyhow::{Context, Result};
use tracing::info;

use borg_core::{
    repository::{Repository, RepositoryConfig},
    storage::{parse_storage_config, build_operator, StorageConfig},
};

use super::get_passphrase;
use crate::{Cli, InitArgs};

pub async fn run(cli: &Cli, args: &InitArgs) -> Result<()> {
    let repo_str = if let Some(url) = &args.webdav_url {
        normalize_webdav_url(url, "webdav", args.webdav_user.as_deref(), args.webdav_pass.as_deref())?
    } else if let Some(url) = &args.webdavs_url {
        normalize_webdav_url(url, "webdavs", args.webdav_user.as_deref(), args.webdav_pass.as_deref())?
    } else {
        super::get_repo_path(cli)?
    };
    let storage_config = parse_storage_config(&repo_str)?;

    // Parse encryption mode (simplified for now)
    let encrypted = match args.encryption.as_str() {
        "none" => false,
        _ => true,
    };

    // Get passphrase if encryption is enabled
    let passphrase = if encrypted {
        let pass1 = get_passphrase("Enter new passphrase: ").await?;
        let pass2 = get_passphrase("Enter same passphrase again: ").await?;

        if pass1 != pass2 {
            anyhow::bail!("Passphrases do not match");
        }

        if pass1.is_empty() {
            anyhow::bail!("Passphrase cannot be empty");
        }

        Some(pass1)
    } else {
        None
    };

    // Create parent directories if requested (only for local storage)
    if args.make_parent_dirs {
        if let StorageConfig::Local { path } = &storage_config {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .context("Failed to create parent directories")?;
            }
        }
    }

    // Initialize repository
    info!("Initializing repository at {}", repo_str);

    let mut config = RepositoryConfig::default();
    config.encrypted = encrypted;

    let op = build_operator(storage_config)?;
    let _repo = Repository::init(
        op,
        passphrase.as_deref(),
        Some(config)
    ).await.context("Failed to initialize repository")?;

    println!("Initialized repository at {}", repo_str);

    if encrypted {
        println!();
        println!("IMPORTANT: Keep your passphrase safe! Without it, you cannot access your backups.");
    }

    Ok(())
}

/// Parse human-readable size string (e.g., "100G", "500M")
fn parse_size(s: &str) -> Result<u64> {
    let s = s.trim();
    let (num, unit) = if s.chars().last().map(|c| c.is_alphabetic()).unwrap_or(false) {
        let idx = s.len() - 1;
        let unit_str = &s[idx..];
        // Check for two-char units like "GB"
        if s.len() > 1 && s.chars().nth(s.len() - 2).map(|c| c.is_alphabetic()).unwrap_or(false) {
            (&s[..s.len()-2], &s[s.len()-2..])
        } else {
            (&s[..idx], unit_str)
        }
    } else {
        (s, "")
    };

    let num: u64 = num.parse()
        .context("Invalid size number")?;

    let multiplier: u64 = match unit.to_uppercase().as_str() {
        "" | "B" => 1,
        "K" | "KB" | "KIB" => 1024,
        "M" | "MB" | "MIB" => 1024 * 1024,
        "G" | "GB" | "GIB" => 1024 * 1024 * 1024,
        "T" | "TB" | "TIB" => 1024 * 1024 * 1024 * 1024,
        other => anyhow::bail!("Unknown size unit: {}", other),
    };

    Ok(num * multiplier)
}

fn normalize_webdav_url(
    value: &str,
    scheme: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<String> {
    let trimmed = value.trim();
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.starts_with("webdav://")
        || trimmed.starts_with("webdavs://")
        || trimmed.starts_with("dav://")
        || trimmed.starts_with("davs://")
    {
        trimmed.to_string()
    } else {
        format!("{}://{}", scheme, trimmed)
    };

    let mut url = url::Url::parse(&with_scheme)
        .map_err(|e| anyhow::anyhow!("Invalid WebDAV URL '{}': {}", with_scheme, e))?;

    if let Some(user) = username {
        url.set_username(user)
            .map_err(|_| anyhow::anyhow!("Invalid WebDAV username"))?;
    }
    if let Some(pass) = password {
        url.set_password(Some(pass))
            .map_err(|_| anyhow::anyhow!("Invalid WebDAV password"))?;
    }

    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("100").unwrap(), 100);
        assert_eq!(parse_size("100B").unwrap(), 100);
        assert_eq!(parse_size("1K").unwrap(), 1024);
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("100M").unwrap(), 100 * 1024 * 1024);
        assert_eq!(parse_size("10G").unwrap(), 10 * 1024 * 1024 * 1024);
    }
}
