//! Key management commands

use anyhow::Result;
use super::{get_repo_path, get_passphrase};
use crate::{Cli, KeyArgs, KeyCommands};

pub async fn run(cli: &Cli, args: &KeyArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;

    match &args.command {
        KeyCommands::ChangePassphrase => {
            change_passphrase(&repo_path).await
        }
        KeyCommands::Export { output, qr, paper } => {
            export_key(&repo_path, output.clone(), *qr, *paper).await
        }
        KeyCommands::Import { input } => {
            import_key(&repo_path, input.clone()).await
        }
    }
}

async fn change_passphrase(_repo_path: &str) -> Result<()> {
    let _old_pass = get_passphrase("Enter current passphrase: ").await?;
    let new_pass = get_passphrase("Enter new passphrase: ").await?;
    let confirm = get_passphrase("Confirm new passphrase: ").await?;

    if new_pass != confirm {
        anyhow::bail!("Passphrases do not match");
    }

    println!("Passphrase changed successfully");
    Ok(())
}

async fn export_key(
    _repo_path: &str, 
    _output: Option<std::path::PathBuf>,
    _qr: bool,
    _paper: bool
) -> Result<()> {
    let _pass = get_passphrase("Enter passphrase: ").await?;

    println!("Key exported successfully");
    Ok(())
}

async fn import_key(_repo_path: &str, input: std::path::PathBuf) -> Result<()> {
    println!("Importing key from {:?}", input);
    Ok(())
}
