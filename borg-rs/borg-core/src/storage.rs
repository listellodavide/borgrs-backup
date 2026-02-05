//! Storage backend abstraction using Apache OpenDAL.

use crate::error::{BorgError, Result};
use opendal::{services, Operator};
use url::Url;
use serde::{Deserialize, Serialize};
/// Configuration for a storage backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageConfig {
    /// Local filesystem storage.
    Local { path: std::path::PathBuf },
    /// WebDAV storage.
    WebDav {
        endpoint: String,
        root: String,
        username: Option<String>,
        password: Option<String>,
    },
}

impl Default for StorageConfig {
    fn default() -> Self {
        StorageConfig::Local {
            path: std::path::PathBuf::from("repo"),
        }
    }
}

pub fn build_operator(config: StorageConfig) -> Result<Operator> {
    match config {
        StorageConfig::Local { path } => {
            let builder = services::Fs::default()
                .root(&path.to_string_lossy());
            
            Ok(Operator::new(builder)
                .map_err(|e| BorgError::Repository(format!("Failed to create local storage: {}", e)))?
                .finish())
        }
        StorageConfig::WebDav { endpoint, root, username, password } => {
            let mut builder = services::Webdav::default()
                .endpoint(&endpoint)
                .root(&root);
            
            if let Some(user) = username {
                builder = builder.username(&user);
            }
            if let Some(pass) = password {
                builder = builder.password(&pass);
            }

            Ok(Operator::new(builder)
                .map_err(|e| BorgError::Repository(format!("Failed to create WebDAV storage: {}", e)))?
                .finish())
        }
    }
}

/// Parse a repository string into a StorageConfig.
/// 
/// Supported formats:
/// - `/path/to/repo` (Local)
/// - `dav://user:pass@host:port/path` (WebDAV)
/// - `webdav://host/path` (WebDAV)
pub fn parse_storage_config(repo_str: &str) -> Result<StorageConfig> {
    if repo_str.starts_with("dav://")
        || repo_str.starts_with("davs://")
        || repo_str.starts_with("webdav://")
        || repo_str.starts_with("webdavs://")
        || repo_str.starts_with("http://")
        || repo_str.starts_with("https://")
    {
        let url = Url::parse(repo_str)
            .map_err(|e| BorgError::InvalidArgument(format!("Invalid WebDAV URL: {}", e)))?;
        
        let scheme = if url.scheme().contains("s") { "https" } else { "http" };
        let host = url.host_str().ok_or_else(|| BorgError::InvalidArgument("Missing host in WebDAV URL".to_string()))?;
        let port = url.port();
        
        let endpoint = if let Some(p) = port {
            format!("{}://{}:{}", scheme, host, p)
        } else {
            format!("{}://{}", scheme, host)
        };
        
        let root = url.path().to_string();
        let username = if url.username().is_empty() { None } else { Some(url.username().to_string()) };
        let password = url.password().map(|p| p.to_string());
        
        Ok(StorageConfig::WebDav {
            endpoint,
            root,
            username,
            password,
        })
    } else {
        // Assume local path
        Ok(StorageConfig::Local {
            path: std::path::PathBuf::from(repo_str),
        })
    }
}
