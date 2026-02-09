use anyhow::Result;

pub async fn create_archive(repo_path: &str) -> Result<()> {
    // Placeholder for borg-core integration
    println!("Creating archive for repo: {}", repo_path);
    Ok(())
}

pub async fn list_archives(repo_path: &str) -> Result<Vec<String>> {
    // Placeholder for borg-core integration
    println!("Listing archives for repo: {}", repo_path);
    Ok(vec!["daily-2023-10-24".to_string(), "daily-2023-10-23".to_string()])
}
