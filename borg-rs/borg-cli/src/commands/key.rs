//! Key management commands

use anyhow::{Context, Result};
use super::{get_repo_path, get_passphrase};
use crate::{Cli, KeyArgs, KeyCommands};
use borg_core::repository::Repository;
use borg_core::storage::{parse_storage_config, build_operator};
use std::path::PathBuf;

const KEY_EXPORT_HEADER: &str = "--- BEGIN BORG-RS RECOVERY KEY ---";
const KEY_EXPORT_FOOTER: &str = "--- END BORG-RS RECOVERY KEY ---";

pub async fn run(cli: &Cli, args: &KeyArgs) -> Result<()> {
    let repo_path_raw = get_repo_path(cli)?;

    match &args.command {
        KeyCommands::ChangePassphrase => {
            change_passphrase(&repo_path_raw).await
        }
        KeyCommands::Export { output, qr, paper } => {
            export_key(&repo_path_raw, output.clone(), *qr, *paper).await
        }
        KeyCommands::Import { input } => {
            import_key(&repo_path_raw, input.clone()).await
        }
    }
}

async fn change_passphrase(repo_path: &str) -> Result<()> {
    let old_pass = get_passphrase("Enter current passphrase: ").await?;
    let new_pass = get_passphrase("Enter new passphrase: ").await?;
    let confirm = get_passphrase("Confirm new passphrase: ").await?;

    if new_pass != confirm {
        anyhow::bail!("New passphrases do not match");
    }

    let config = parse_storage_config(repo_path)?;
    let op = build_operator(config)?;
    let mut repo = Repository::open(op, repo_path.to_string(), Some(&old_pass)).await?;

    // Export the raw key using the old passphrase
    let key_bytes = repo.export_key()?;

    // Re-import (re-encrypt) the key with the new passphrase
    repo.import_key(&key_bytes, &new_pass).await?;

    println!("Passphrase changed successfully");
    Ok(())
}

async fn export_key(
    repo_path: &str,
    output: Option<PathBuf>,
    _qr: bool,
    _paper: bool
) -> Result<()> {
    let passphrase = get_passphrase("Enter passphrase for repository: ").await?;

    let config = parse_storage_config(repo_path)?;
    let op = build_operator(config)?;
    let repo = Repository::open(op, repo_path.to_string(), Some(&passphrase)).await?;

    let key_bytes = repo.export_key()?;
    let encoded_key = hex::encode(&key_bytes);

    let output_content = format!(
        "{}\n{}\n{}",
        KEY_EXPORT_HEADER, encoded_key, KEY_EXPORT_FOOTER
    );

    if let Some(path) = output {
        tokio::fs::write(&path, output_content).await?;
        println!("Repository key exported to {}", path.display());
    } else {
        println!("{}", output_content);
    }

    if _qr {
        println!("QR code generation is not yet implemented.");
    }
    if _paper {
        println!("Paper key formatting is not yet implemented.");
    }

    Ok(())
}

async fn import_key(repo_path: &str, input: PathBuf) -> Result<()> {
    println!("Importing key from {}", input.display());

    let key_content = tokio::fs::read_to_string(&input)
        .await
        .context("Failed to read key file")?;

    let mut lines = key_content.lines();
    let header = lines.next().unwrap_or_default();
    let encoded_key = lines.next().unwrap_or_default();
    let footer = lines.next().unwrap_or_default();

    if header != KEY_EXPORT_HEADER || footer != KEY_EXPORT_FOOTER {
        anyhow::bail!("Invalid key file format. Missing header or footer.");
    }

    let key_bytes = hex::decode(encoded_key).context("Failed to decode key from hex")?;

    let new_passphrase = get_passphrase("Enter NEW passphrase for repository: ").await?;
    let confirm_passphrase = get_passphrase("Confirm NEW passphrase: ").await?;

    if new_passphrase != confirm_passphrase {
        anyhow::bail!("New passphrases do not match");
    }

    let config = parse_storage_config(repo_path)?;
    let op = build_operator(config)?;

    // Open repository without passphrase (recovery mode)
    let mut repo = Repository::open(op, repo_path.to_string(), None).await?;

    repo.import_key(&key_bytes, &new_passphrase).await?;

    println!("Repository key imported and re-encrypted with new passphrase successfully.");
    Ok(())
}
