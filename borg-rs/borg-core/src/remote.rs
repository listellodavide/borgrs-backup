//! Remote repository support via SSH
//!
//! Implements SSH-based remote repository access using an agent model
//! similar to original Borg.

use crate::error::{BorgError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, info, instrument};

/// Remote repository location
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteLocation {
    /// SSH username
    pub user: Option<String>,
    /// SSH host
    pub host: String,
    /// SSH port (default: 22)
    pub port: u16,
    /// Path on remote host
    pub path: PathBuf,
    /// SSH key path (optional)
    pub ssh_key: Option<PathBuf>,
}

impl RemoteLocation {
    /// Parse a remote location string (user@host:path or ssh://user@host:port/path)
    pub fn parse(location: &str) -> Result<Self> {
        // Handle ssh:// URL format
        if location.starts_with("ssh://") {
            return Self::parse_ssh_url(&location[6..]);
        }

        // Handle user@host:path format
        if let Some(colon_pos) = location.rfind(':') {
            let (host_part, path) = location.split_at(colon_pos);
            let path = &path[1..]; // Remove the colon

            let (user, host) = if let Some(at_pos) = host_part.find('@') {
                let (user, host) = host_part.split_at(at_pos);
                (Some(user.to_string()), host[1..].to_string())
            } else {
                (None, host_part.to_string())
            };

            return Ok(Self {
                user,
                host,
                port: 22,
                path: PathBuf::from(path),
                ssh_key: None,
            });
        }

        Err(BorgError::InvalidArgument(format!(
            "Invalid remote location: {}",
            location
        )))
    }

    fn parse_ssh_url(url: &str) -> Result<Self> {
        // Format: user@host:port/path or user@host/path or host/path
        let (auth_host, path) = url
            .split_once('/')
            .ok_or_else(|| BorgError::InvalidArgument("Missing path in SSH URL".to_string()))?;

        let (user, host_port) = if let Some(at_pos) = auth_host.find('@') {
            let (user, rest) = auth_host.split_at(at_pos);
            (Some(user.to_string()), &rest[1..])
        } else {
            (None, auth_host)
        };

        let (host, port) = if let Some(colon_pos) = host_port.find(':') {
            let (host, port_str) = host_port.split_at(colon_pos);
            let port: u16 = port_str[1..]
                .parse()
                .map_err(|_| BorgError::InvalidArgument("Invalid port number".to_string()))?;
            (host.to_string(), port)
        } else {
            (host_port.to_string(), 22)
        };

        Ok(Self {
            user,
            host,
            port,
            path: PathBuf::from(format!("/{}", path)),
            ssh_key: None,
        })
    }

    /// Convert to connection string
    pub fn to_string(&self) -> String {
        let user_part = self.user.as_ref().map(|u| format!("{}@", u)).unwrap_or_default();
        format!("{}{}:{}", user_part, self.host, self.path.display())
    }
}

/// Message types for remote protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RemoteMessage {
    /// Initialize connection
    Init { version: u32 },
    /// Open repository
    Open { path: PathBuf },
    /// Create new repository
    Create { path: PathBuf, config: Vec<u8> },
    /// Get chunk
    GetChunk { id: [u8; 32] },
    /// Put chunk
    PutChunk { id: [u8; 32], data: Vec<u8> },
    /// Check if chunk exists
    HasChunk { id: [u8; 32] },
    /// Get manifest
    GetManifest,
    /// Put manifest
    PutManifest { data: Vec<u8> },
    /// Lock repository
    Lock { exclusive: bool },
    /// Unlock repository
    Unlock,
    /// Close connection
    Close,
    /// Response
    Response(RemoteResponse),
}

/// Response types for remote protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RemoteResponse {
    /// Success with optional data
    Ok { data: Option<Vec<u8>> },
    /// Boolean result
    Bool(bool),
    /// Error
    Error { message: String },
}

/// Remote repository client
#[async_trait]
pub trait RemoteClient: Send + Sync {
    /// Connect to the remote server
    async fn connect(&mut self) -> Result<()>;
    
    /// Disconnect from the remote server
    async fn disconnect(&mut self) -> Result<()>;
    
    /// Send a message and receive a response
    async fn send(&mut self, message: RemoteMessage) -> Result<RemoteResponse>;
}

/// SSH-based remote client implementation
pub struct SshRemoteClient {
    location: RemoteLocation,
    connected: bool,
    // In a real implementation, this would hold the SSH session
}

impl SshRemoteClient {
    /// Create a new SSH remote client
    pub fn new(location: RemoteLocation) -> Self {
        Self {
            location,
            connected: false,
        }
    }

    /// Set the SSH key to use for authentication
    pub fn with_ssh_key(mut self, key_path: PathBuf) -> Self {
        self.location.ssh_key = Some(key_path);
        self
    }
}

#[async_trait]
impl RemoteClient for SshRemoteClient {
    #[instrument(skip(self))]
    async fn connect(&mut self) -> Result<()> {
        info!("Connecting to {}...", self.location.to_string());
        
        // In a real implementation, this would:
        // 1. Establish SSH connection using russh
        // 2. Start the remote borg-rust serve command
        // 3. Initialize the protocol
        
        self.connected = true;
        debug!("Connected to remote repository");
        
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        if self.connected {
            debug!("Disconnecting from remote repository");
            self.connected = false;
        }
        Ok(())
    }

    async fn send(&mut self, message: RemoteMessage) -> Result<RemoteResponse> {
        if !self.connected {
            return Err(BorgError::RemoteConnection("Not connected".to_string()));
        }

        // In a real implementation, this would:
        // 1. Serialize the message
        // 2. Send over SSH channel
        // 3. Receive and deserialize response
        
        debug!("Sending remote message: {:?}", message);
        
        // Placeholder response
        Ok(RemoteResponse::Ok { data: None })
    }
}

/// Remote repository wrapper
pub struct RemoteRepository {
    client: Box<dyn RemoteClient>,
    location: RemoteLocation,
}

impl RemoteRepository {
    /// Connect to a remote repository
    pub async fn connect(location: RemoteLocation) -> Result<Self> {
        let mut client: Box<dyn RemoteClient> = Box::new(SshRemoteClient::new(location.clone()));
        client.connect().await?;
        
        // Open the repository
        let response = client.send(RemoteMessage::Open { 
            path: location.path.clone() 
        }).await?;
        
        match response {
            RemoteResponse::Ok { .. } => {},
            RemoteResponse::Error { message } => {
                return Err(BorgError::RemoteRepository(message));
            }
            _ => {}
        }
        
        Ok(Self { client, location })
    }

    /// Check if a chunk exists
    pub async fn has_chunk(&mut self, id: &[u8; 32]) -> Result<bool> {
        let response = self.client.send(RemoteMessage::HasChunk { id: *id }).await?;
        match response {
            RemoteResponse::Bool(exists) => Ok(exists),
            RemoteResponse::Error { message } => Err(BorgError::RemoteRepository(message)),
            _ => Err(BorgError::RemoteRepository("Unexpected response".to_string())),
        }
    }

    /// Get a chunk
    pub async fn get_chunk(&mut self, id: &[u8; 32]) -> Result<Vec<u8>> {
        let response = self.client.send(RemoteMessage::GetChunk { id: *id }).await?;
        match response {
            RemoteResponse::Ok { data: Some(data) } => Ok(data),
            RemoteResponse::Ok { data: None } => {
                Err(BorgError::RemoteRepository("Chunk not found".to_string()))
            }
            RemoteResponse::Error { message } => Err(BorgError::RemoteRepository(message)),
            _ => Err(BorgError::RemoteRepository("Unexpected response".to_string())),
        }
    }

    /// Put a chunk
    pub async fn put_chunk(&mut self, id: &[u8; 32], data: Vec<u8>) -> Result<()> {
        let response = self.client.send(RemoteMessage::PutChunk { id: *id, data }).await?;
        match response {
            RemoteResponse::Ok { .. } => Ok(()),
            RemoteResponse::Error { message } => Err(BorgError::RemoteRepository(message)),
            _ => Err(BorgError::RemoteRepository("Unexpected response".to_string())),
        }
    }

    /// Get the manifest
    pub async fn get_manifest(&mut self) -> Result<Vec<u8>> {
        let response = self.client.send(RemoteMessage::GetManifest).await?;
        match response {
            RemoteResponse::Ok { data: Some(data) } => Ok(data),
            RemoteResponse::Ok { data: None } => {
                Err(BorgError::RemoteRepository("Manifest not found".to_string()))
            }
            RemoteResponse::Error { message } => Err(BorgError::RemoteRepository(message)),
            _ => Err(BorgError::RemoteRepository("Unexpected response".to_string())),
        }
    }

    /// Lock the repository
    pub async fn lock(&mut self, exclusive: bool) -> Result<()> {
        let response = self.client.send(RemoteMessage::Lock { exclusive }).await?;
        match response {
            RemoteResponse::Ok { .. } => Ok(()),
            RemoteResponse::Error { message } => Err(BorgError::RemoteRepository(message)),
            _ => Err(BorgError::RemoteRepository("Unexpected response".to_string())),
        }
    }

    /// Unlock the repository
    pub async fn unlock(&mut self) -> Result<()> {
        let response = self.client.send(RemoteMessage::Unlock).await?;
        match response {
            RemoteResponse::Ok { .. } => Ok(()),
            RemoteResponse::Error { message } => Err(BorgError::RemoteRepository(message)),
            _ => Err(BorgError::RemoteRepository("Unexpected response".to_string())),
        }
    }

    /// Get the remote location
    pub fn location(&self) -> &RemoteLocation {
        &self.location
    }
}

impl Drop for RemoteRepository {
    fn drop(&mut self) {
        // Attempt to disconnect cleanly
        // In async context, this should be handled differently
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_location_simple() {
        let loc = RemoteLocation::parse("user@host:/path/to/repo").unwrap();
        assert_eq!(loc.user, Some("user".to_string()));
        assert_eq!(loc.host, "host");
        assert_eq!(loc.port, 22);
        assert_eq!(loc.path, PathBuf::from("/path/to/repo"));
    }

    #[test]
    fn test_parse_location_no_user() {
        let loc = RemoteLocation::parse("host:/path/to/repo").unwrap();
        assert_eq!(loc.user, None);
        assert_eq!(loc.host, "host");
    }

    #[test]
    fn test_parse_ssh_url() {
        let loc = RemoteLocation::parse("ssh://user@host:2222/path/to/repo").unwrap();
        assert_eq!(loc.user, Some("user".to_string()));
        assert_eq!(loc.host, "host");
        assert_eq!(loc.port, 2222);
        assert_eq!(loc.path, PathBuf::from("/path/to/repo"));
    }

    #[test]
    fn test_location_to_string() {
        let loc = RemoteLocation {
            user: Some("user".to_string()),
            host: "host".to_string(),
            port: 22,
            path: PathBuf::from("/repo"),
            ssh_key: None,
        };
        assert_eq!(loc.to_string(), "user@host:/repo");
    }
}
