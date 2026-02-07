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
    /// S3-compatible storage (AWS S3, MinIO, etc.)
    S3 {
        bucket: String,
        prefix: String,
        region: Option<String>,
        endpoint: Option<String>,
        access_key: Option<String>,
        secret_key: Option<String>,
        session_token: Option<String>,
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
        StorageConfig::S3 { bucket, prefix, region, endpoint, access_key, secret_key, session_token } => {
            // Configure S3-compatible backend. Endpoint can be provided for MinIO or custom S3-compatible
            // servers. Access keys may be provided in the URL or environment.
            let mut builder = services::S3::default().bucket(&bucket).root(&prefix);

            if let Some(r) = region {
                builder = builder.region(&r);
            }
            if let Some(ep) = endpoint {
                builder = builder.endpoint(&ep);
            }
            // Prefer explicit credentials provided in the StorageConfig, otherwise fall back
            // to environment variables (S3_ACCESS_KEY, S3_SECRET_KEY, S3_SESSION_TOKEN).
            if let Some(ak) = access_key {
                builder = builder.access_key_id(&ak);
            } else if let Ok(ak_env) = std::env::var("S3_ACCESS_KEY") {
                if !ak_env.is_empty() {
                    builder = builder.access_key_id(&ak_env);
                }
            }

            if let Some(sk) = secret_key {
                builder = builder.secret_access_key(&sk);
            } else if let Ok(sk_env) = std::env::var("S3_SECRET_KEY") {
                if !sk_env.is_empty() {
                    builder = builder.secret_access_key(&sk_env);
                }
            }

            if let Some(tok) = session_token {
                builder = builder.session_token(&tok);
            } else if let Ok(tok_env) = std::env::var("S3_SESSION_TOKEN") {
                if !tok_env.is_empty() {
                    builder = builder.session_token(&tok_env);
                }
            } else if let Ok(tok_env) = std::env::var("AWS_SESSION_TOKEN") {
                if !tok_env.is_empty() {
                    builder = builder.session_token(&tok_env);
                }
            }

            Ok(Operator::new(builder)
                .map_err(|e| BorgError::Repository(format!("Failed to create S3 storage: {}", e)))?
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

        // Heuristic: if the URL contains userinfo (username/password) and a path with at least
        // one segment, treat it as an S3-style endpoint expressed over HTTPS, e.g.
        // `https://ACCESS:SECRET@endpoint:9000/bucket/prefix` -> S3 (MinIO style)
        if (url.scheme() == "https" || url.scheme() == "http") && !url.username().is_empty() {
            // Extract endpoint (scheme://host[:port])
            let host = url.host_str().ok_or_else(|| BorgError::InvalidArgument("Missing host in URL".to_string()))?;
            let port = url.port();
            let endpoint = if let Some(p) = port {
                format!("{}://{}:{}", url.scheme(), host, p)
            } else {
                format!("{}://{}", url.scheme(), host)
            };

            // Path: /bucket[/prefix...]
            let path = url.path().trim_start_matches('/');
            let mut segments = path.splitn(2, '/');
            let bucket = segments.next().unwrap_or("").to_string();
            let prefix = segments.next().map(|s| s.to_string()).unwrap_or_default();

            let access_key = if url.username().is_empty() { None } else { Some(url.username().to_string()) };
            let secret_key = url.password().map(|p| p.to_string());

            return Ok(StorageConfig::S3 {
                bucket,
                prefix,
                region: None,
                endpoint: Some(endpoint),
                access_key,
                secret_key,
                session_token: None,
            });
        }

        // Otherwise, default to WebDAV interpretation
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
    } else if repo_str.starts_with("s3://") || repo_str.starts_with("s3s://") {
        // s3://bucket[/prefix] or s3s://bucket[/prefix]
        let url = Url::parse(repo_str)
            .map_err(|e| BorgError::InvalidArgument(format!("Invalid S3 URL: {}", e)))?;

        let bucket = url.host_str().ok_or_else(|| BorgError::InvalidArgument("Missing bucket in S3 URL".to_string()))?.to_string();
        // path() returns leading '/'
        let prefix = if url.path() == "/" { "".to_string() } else { url.path().trim_start_matches('/').to_string() };

        // Credentials may be embedded as user:pass@ in the URL
        let access_key = if url.username().is_empty() { None } else { Some(url.username().to_string()) };
        let secret_key = url.password().map(|p| p.to_string());

        // Allow custom endpoint via host if using non-standard S3 scheme (s3s://my-minio:9000/bucket)
        // For s3://bucket..., host is bucket; endpoint will be left None so SDK picks defaults.
        // If user supplies an explicit endpoint via query or HTTP(S) URL, they should use an https:// URL instead.
        let endpoint = None;

        Ok(StorageConfig::S3 {
            bucket,
            prefix,
            region: None,
            endpoint,
            access_key,
            secret_key,
            session_token: None,
        })
    } else {
        // Assume local path
        Ok(StorageConfig::Local {
            path: std::path::PathBuf::from(repo_str),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minio_https_with_creds() {
        let uri = "https://ACCESS:SECRET@minio.local:9000/mybucket/prefix/path";
        let cfg = parse_storage_config(uri).expect("parse failed");
        match cfg {
            StorageConfig::S3 { bucket, prefix, endpoint, access_key, secret_key, .. } => {
                assert_eq!(bucket, "mybucket");
                assert!(prefix.starts_with("prefix"));
                assert_eq!(access_key.unwrap(), "ACCESS");
                assert_eq!(secret_key.unwrap(), "SECRET");
                assert!(endpoint.unwrap().starts_with("https://minio.local:9000"));
            }
            other => panic!("expected S3 config, got {:?}", other),
        }
    }

    #[test]
    fn parse_s3_scheme_bucket() {
        let uri = "s3://mybucket/some/prefix";
        let cfg = parse_storage_config(uri).expect("parse failed");
        match cfg {
            StorageConfig::S3 { bucket, prefix, access_key, secret_key, endpoint, .. } => {
                assert_eq!(bucket, "mybucket");
                assert_eq!(prefix, "some/prefix");
                assert!(access_key.is_none());
                assert!(secret_key.is_none());
                assert!(endpoint.is_none());
            }
            other => panic!("expected S3 config, got {:?}", other),
        }
    }

    #[test]
    fn parse_local_path() {
        let path = "/tmp/repo";
        let cfg = parse_storage_config(path).expect("parse failed");
        match cfg {
            StorageConfig::Local { path: p } => {
                assert_eq!(p.to_string_lossy(), "/tmp/repo");
            }
            other => panic!("expected Local config, got {:?}", other),
        }
    }

    #[test]
    fn parse_webdav_url() {
        let uri = "dav://user:pass@example.com/dav/repo";
        let cfg = parse_storage_config(uri).expect("parse failed");
        match cfg {
            StorageConfig::WebDav { endpoint, root, username, password } => {
                assert_eq!(endpoint, "http://example.com");
                assert_eq!(root, "/dav/repo");
                assert_eq!(username.unwrap(), "user");
                assert_eq!(password.unwrap(), "pass");
            }
            other => panic!("expected WebDav config, got {:?}", other),
        }
    }
}
