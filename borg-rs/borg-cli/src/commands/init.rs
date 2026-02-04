//! Repository initialization command

use std::path::PathBuf;
use anyhow::{Context, Result};
use tracing::info;

use borg_core::{
    repository::{Repository, RepositoryConfig},
};

use super::get_passphrase;
use crate::{Cli, InitArgs};

pub async fn run(cli: &Cli, args: &InitArgs) -> Result<()> {
    let repo_path = super::get_repo_path(cli)?;

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

    // Create parent directories if requested
    if args.make_parent_dirs {
        if let Some(parent) = PathBuf::from(&repo_path).parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create parent directories")?;
        }
    }

    // Initialize repository
    info!("Initializing repository at {}", repo_path);

    let mut config = RepositoryConfig::default();
    config.encrypted = encrypted;

    let _repo = Repository::init(
        std::path::Path::new(&repo_path),
        passphrase.as_deref(),
        Some(config)
    ).context("Failed to initialize repository")?;

    println!("Initialized repository at {}", repo_path);

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
