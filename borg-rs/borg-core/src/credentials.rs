//! Secure credential storage using the system keyring
use anyhow::{Context, Result};
use keyring::Entry;
use uuid::Uuid;

const KEYRING_SERVICE: &str = "borg-rs";

/// Store a password in the keyring for a given repository UUID
pub fn store_password(repo_uuid: &Uuid, password: &str) -> Result<()> {
    let entry = Entry::new(KEYRING_SERVICE, &repo_uuid.to_string())
        .context("Failed to create keyring entry")?;
    entry
        .set_password(password)
        .context("Failed to set password in keyring")?;
    Ok(())
}

/// Retrieve a password from the keyring for a given repository UUID
pub fn get_password(repo_uuid: &Uuid) -> Result<String> {
    let entry = Entry::new(KEYRING_SERVICE, &repo_uuid.to_string())
        .context("Failed to create keyring entry")?;
    entry
        .get_password()
        .context(format!("No password found in keyring for repository {}", repo_uuid))
}

/// Delete a password from the keyring for a given repository UUID
pub fn delete_password(repo_uuid: &Uuid) -> Result<()> {
    let entry = Entry::new(KEYRING_SERVICE, &repo_uuid.to_string())
        .context("Failed to create keyring entry")?;
    entry
        .delete_password()
        .context("Failed to delete password from keyring")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_store_and_get_password() {
        // Use a unique UUID for this test to avoid interfering with other tests or real data.
        let repo_uuid = Uuid::new_v4();
        let password = "my-super-secret-dummy-password-for-testing";

        // 1. Store the password
        let store_result = store_password(&repo_uuid, password);
        assert!(store_result.is_ok(), "Failed to store password: {:?}", store_result.err());

        // 2. Retrieve the password
        let get_result = get_password(&repo_uuid);
        assert!(get_result.is_ok(), "Failed to get password: {:?}", get_result.err());
        let retrieved_password = get_result.unwrap();

        // 3. Validate the content
        assert_eq!(retrieved_password, password);

        // 4. Clean up the entry
        let delete_result = delete_password(&repo_uuid);
        assert!(delete_result.is_ok(), "Failed to delete password: {:?}", delete_result.err());

        // 5. Verify deletion
        let get_after_delete_result = get_password(&repo_uuid);
        assert!(get_after_delete_result.is_err(), "Password was not deleted successfully");
    }
}
