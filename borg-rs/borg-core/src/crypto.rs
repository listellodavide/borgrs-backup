//! Cryptographic operations for Borg-Rust
//!
//! Implements authenticated encryption using AES-256-GCM with
//! key derivation via Argon2id. All data is encrypted client-side
//! before being stored.

use crate::error::{BorgError, Result};
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fmt;
use tracing::{debug, instrument};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Size of the encryption key in bytes (256 bits)
const KEY_SIZE: usize = 32;
/// Size of the nonce/IV in bytes (96 bits for AES-GCM)
const NONCE_SIZE: usize = 12;
/// Size of the HMAC key in bytes
const HMAC_KEY_SIZE: usize = 32;

/// Encryption key that is automatically zeroed on drop
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey {
    /// AES-256 encryption key
    enc_key: [u8; KEY_SIZE],
    /// HMAC-SHA256 key for data integrity
    mac_key: [u8; HMAC_KEY_SIZE],
}

impl EncryptionKey {
    /// Generate a new random encryption key
    pub fn generate() -> Self {
        let mut enc_key = [0u8; KEY_SIZE];
        let mut mac_key = [0u8; HMAC_KEY_SIZE];
        OsRng.fill_bytes(&mut enc_key);
        OsRng.fill_bytes(&mut mac_key);
        Self { enc_key, mac_key }
    }

    /// Derive a key from a passphrase using Argon2id
    #[instrument(skip(passphrase))]
    pub fn from_passphrase(passphrase: &str, salt: &[u8]) -> Result<Self> {
        if passphrase.is_empty() {
            return Err(BorgError::InvalidArgument(
                "Passphrase cannot be empty".to_string(),
            ));
        }

        let salt_string = SaltString::encode_b64(salt)
            .map_err(|e| BorgError::KeyDerivation(format!("Invalid salt: {}", e)))?;

        let argon2 = Argon2::default();

        // Derive 64 bytes: 32 for encryption, 32 for MAC
        let password_hash = argon2
            .hash_password(passphrase.as_bytes(), &salt_string)
            .map_err(|e| BorgError::KeyDerivation(format!("Key derivation failed: {}", e)))?;

        let hash = password_hash.hash.ok_or_else(|| {
            BorgError::KeyDerivation("Failed to get hash output".to_string())
        })?;

        let hash_bytes = hash.as_bytes();
        
        // Use HKDF-like expansion to get enough key material
        let mut enc_key = [0u8; KEY_SIZE];
        let mut mac_key = [0u8; HMAC_KEY_SIZE];
        
        // Simple key expansion using HMAC
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(hash_bytes)
            .map_err(|e| BorgError::KeyDerivation(format!("HMAC init failed: {}", e)))?;
        mac.update(b"encryption");
        let enc_result = mac.finalize().into_bytes();
        enc_key.copy_from_slice(&enc_result[..KEY_SIZE]);

        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(hash_bytes)
            .map_err(|e| BorgError::KeyDerivation(format!("HMAC init failed: {}", e)))?;
        mac.update(b"authentication");
        let mac_result = mac.finalize().into_bytes();
        mac_key.copy_from_slice(&mac_result[..HMAC_KEY_SIZE]);

        debug!("Derived encryption key from passphrase");
        Ok(Self { enc_key, mac_key })
    }

    /// Serialize the key for secure storage
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(KEY_SIZE + HMAC_KEY_SIZE);
        bytes.extend_from_slice(&self.enc_key);
        bytes.extend_from_slice(&self.mac_key);
        bytes
    }

    /// Deserialize a key from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != KEY_SIZE + HMAC_KEY_SIZE {
            return Err(BorgError::InvalidArgument(format!(
                "Invalid key length: expected {}, got {}",
                KEY_SIZE + HMAC_KEY_SIZE,
                bytes.len()
            )));
        }
        let mut enc_key = [0u8; KEY_SIZE];
        let mut mac_key = [0u8; HMAC_KEY_SIZE];
        enc_key.copy_from_slice(&bytes[..KEY_SIZE]);
        mac_key.copy_from_slice(&bytes[KEY_SIZE..]);
        Ok(Self { enc_key, mac_key })
    }
}

impl fmt::Debug for EncryptionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptionKey")
            .field("enc_key", &"[REDACTED]")
            .field("mac_key", &"[REDACTED]")
            .finish()
    }
}

/// Salt for key derivation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeySalt(pub [u8; 16]);

impl KeySalt {
    /// Generate a new random salt
    pub fn generate() -> Self {
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);
        Self(salt)
    }

    /// Create from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 16 {
            return Err(BorgError::InvalidArgument(
                "Salt must be 16 bytes".to_string(),
            ));
        }
        let mut salt = [0u8; 16];
        salt.copy_from_slice(bytes);
        Ok(Self(salt))
    }
}

/// Encrypted data with nonce and authentication tag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedData {
    /// Nonce/IV for AES-GCM
    pub nonce: [u8; NONCE_SIZE],
    /// Ciphertext with authentication tag
    pub ciphertext: Vec<u8>,
    /// HMAC for additional integrity verification
    pub hmac: [u8; 32],
}

/// Cryptographic provider for encryption/decryption operations
pub struct CryptoProvider {
    key: EncryptionKey,
    cipher: Aes256Gcm,
}

impl CryptoProvider {
    /// Create a new crypto provider with the given key
    pub fn new(key: EncryptionKey) -> Self {
        let cipher = Aes256Gcm::new_from_slice(&key.enc_key)
            .expect("Invalid key size"); // Should never fail with correct key size
        Self { key, cipher }
    }

    /// Create a crypto provider from a passphrase
    pub fn from_passphrase(passphrase: &str, salt: &KeySalt) -> Result<Self> {
        let key = EncryptionKey::from_passphrase(passphrase, &salt.0)?;
        Ok(Self::new(key))
    }

    /// Encrypt data with authenticated encryption
    #[instrument(skip(self, plaintext), fields(plaintext_len = plaintext.len()))]
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedData> {
        // Generate random nonce
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Encrypt with AES-GCM (includes authentication tag)
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| BorgError::Encryption(format!("Encryption failed: {}", e)))?;

        // Compute additional HMAC over nonce + ciphertext
        let hmac = self.compute_hmac(&nonce_bytes, &ciphertext)?;

        debug!(
            "Encrypted {} bytes -> {} bytes",
            plaintext.len(),
            ciphertext.len()
        );

        Ok(EncryptedData {
            nonce: nonce_bytes,
            ciphertext,
            hmac,
        })
    }

    /// Decrypt data and verify authenticity
    #[instrument(skip(self, encrypted), fields(ciphertext_len = encrypted.ciphertext.len()))]
    pub fn decrypt(&self, encrypted: &EncryptedData) -> Result<Vec<u8>> {
        // Verify HMAC first
        let expected_hmac = self.compute_hmac(&encrypted.nonce, &encrypted.ciphertext)?;
        if !constant_time_eq(&encrypted.hmac, &expected_hmac) {
            return Err(BorgError::IntegrityCheck {
                expected: hex_encode(&expected_hmac),
                actual: hex_encode(&encrypted.hmac),
            });
        }

        // Decrypt with AES-GCM
        let nonce = Nonce::from_slice(&encrypted.nonce);
        let plaintext = self
            .cipher
            .decrypt(nonce, encrypted.ciphertext.as_ref())
            .map_err(|_| BorgError::Decryption("Decryption failed - data may be corrupted".to_string()))?;

        debug!(
            "Decrypted {} bytes -> {} bytes",
            encrypted.ciphertext.len(),
            plaintext.len()
        );

        Ok(plaintext)
    }

    /// Compute HMAC-SHA256 over nonce and ciphertext
    fn compute_hmac(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<[u8; 32]> {
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(&self.key.mac_key)
            .map_err(|e| BorgError::Encryption(format!("HMAC init failed: {}", e)))?;
        mac.update(nonce);
        mac.update(ciphertext);
        let result = mac.finalize().into_bytes();
        let mut hmac = [0u8; 32];
        hmac.copy_from_slice(&result);
        Ok(hmac)
    }

    /// Get the encryption key (for key storage operations)
    pub fn key(&self) -> &EncryptionKey {
        &self.key
    }
}

/// Constant-time comparison to prevent timing attacks
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

/// Encode bytes as hex string
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Repository key file format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryKey {
    /// Version of the key format
    pub version: u32,
    /// Salt used for key derivation
    pub salt: KeySalt,
    /// Encrypted repository key (encrypted with passphrase-derived key)
    pub encrypted_key: EncryptedData,
    /// Number of Argon2 iterations used
    pub iterations: u32,
}

impl RepositoryKey {
    /// Create a new repository key encrypted with the given passphrase
    pub fn create(passphrase: &str) -> Result<(Self, EncryptionKey)> {
        let salt = KeySalt::generate();
        let repo_key = EncryptionKey::generate();
        
        // Derive key from passphrase
        let passphrase_crypto = CryptoProvider::from_passphrase(passphrase, &salt)?;
        
        // Encrypt the repository key
        let encrypted_key = passphrase_crypto.encrypt(&repo_key.to_bytes())?;
        
        Ok((
            Self {
                version: 1,
                salt,
                encrypted_key,
                iterations: 3, // Argon2 default
            },
            repo_key,
        ))
    }

    /// Wrap an existing key with a new passphrase
    pub fn wrap(key: &EncryptionKey, passphrase: &str) -> Result<Self> {
        let salt = KeySalt::generate();
        let passphrase_crypto = CryptoProvider::from_passphrase(passphrase, &salt)?;
        let encrypted_key = passphrase_crypto.encrypt(&key.to_bytes())?;

        Ok(Self {
            version: 1,
            salt,
            encrypted_key,
            iterations: 3,
        })
    }

    /// Decrypt the repository key using the passphrase
    pub fn decrypt(&self, passphrase: &str) -> Result<EncryptionKey> {
        let passphrase_crypto = CryptoProvider::from_passphrase(passphrase, &self.salt)?;
        let key_bytes = passphrase_crypto.decrypt(&self.encrypted_key)?;
        EncryptionKey::from_bytes(&key_bytes)
    }

    /// Change the passphrase for this key
    pub fn change_passphrase(&mut self, old_passphrase: &str, new_passphrase: &str) -> Result<()> {
        // Decrypt with old passphrase
        let repo_key = self.decrypt(old_passphrase)?;
        
        // Generate new salt and re-encrypt
        self.salt = KeySalt::generate();
        let new_crypto = CryptoProvider::from_passphrase(new_passphrase, &self.salt)?;
        self.encrypted_key = new_crypto.encrypt(&repo_key.to_bytes())?;
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = EncryptionKey::generate();
        let crypto = CryptoProvider::new(key);
        
        let plaintext = b"Hello, Borg-Rust!";
        let encrypted = crypto.encrypt(plaintext).unwrap();
        let decrypted = crypto.decrypt(&encrypted).unwrap();
        
        assert_eq!(plaintext.as_slice(), decrypted.as_slice());
    }

    #[test]
    fn test_key_derivation() {
        let salt = KeySalt::generate();
        let key1 = EncryptionKey::from_passphrase("test-passphrase", &salt.0).unwrap();
        let key2 = EncryptionKey::from_passphrase("test-passphrase", &salt.0).unwrap();
        
        assert_eq!(key1.enc_key, key2.enc_key);
        assert_eq!(key1.mac_key, key2.mac_key);
    }

    #[test]
    fn test_different_passphrases_different_keys() {
        let salt = KeySalt::generate();
        let key1 = EncryptionKey::from_passphrase("passphrase1", &salt.0).unwrap();
        let key2 = EncryptionKey::from_passphrase("passphrase2", &salt.0).unwrap();
        
        assert_ne!(key1.enc_key, key2.enc_key);
    }

    #[test]
    fn test_tampered_data_detected() {
        let key = EncryptionKey::generate();
        let crypto = CryptoProvider::new(key);
        
        let plaintext = b"Sensitive data";
        let mut encrypted = crypto.encrypt(plaintext).unwrap();
        
        // Tamper with ciphertext
        if !encrypted.ciphertext.is_empty() {
            encrypted.ciphertext[0] ^= 0xFF;
        }
        
        // Should fail integrity check
        assert!(crypto.decrypt(&encrypted).is_err());
    }

    #[test]
    fn test_repository_key() {
        let passphrase = "my-secure-passphrase";
        let (repo_key, original_key) = RepositoryKey::create(passphrase).unwrap();
        
        let decrypted_key = repo_key.decrypt(passphrase).unwrap();
        assert_eq!(original_key.enc_key, decrypted_key.enc_key);
        assert_eq!(original_key.mac_key, decrypted_key.mac_key);
    }

    #[test]
    fn test_wrong_passphrase_fails() {
        let (repo_key, _) = RepositoryKey::create("correct-passphrase").unwrap();
        assert!(repo_key.decrypt("wrong-passphrase").is_err());
    }
}
