//! WebDAV client, scanner, and streaming reader.
//!
//! Provides:
//! - Recursive WebDAV directory discovery via PROPFIND
//! - Metadata cache for incremental scans
//! - Streaming WebDAV reader with range/resume support
//! - Content-defined chunking integration for streaming pipelines

use crate::chunker::{ChunkId, Chunker, ChunkerConfig};
use crate::error::{BorgError, Result};
use bytes::Bytes;
use futures_util::{Stream, TryStreamExt};
use quick_xml::de::from_str;
use reqwest::header::{HeaderMap, HeaderValue, IF_RANGE, RANGE};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::io::StreamReader;
use tracing::{instrument, warn};
use url::Url;

const DEFAULT_TIMEOUT_SECS: u64 = 60;
const DEFAULT_MAX_RETRIES: usize = 3;
const DEFAULT_MAX_READERS: usize = 4;
const DEFAULT_READ_BUFFER_SIZE: usize = 256 * 1024;

/// WebDAV authentication configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebDavAuth {
    pub username: String,
    pub password: String,
}

/// WebDAV client configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebDavConfig {
    pub base_url: Url,
    pub root_path: String,
    pub auth: Option<WebDavAuth>,
    pub timeout: Duration,
    pub max_retries: usize,
    pub max_parallel_readers: usize,
    pub read_buffer_size: usize,
}

impl WebDavConfig {
    pub fn new(base_url: Url, root_path: impl Into<String>) -> Self {
        Self {
            base_url,
            root_path: root_path.into(),
            auth: None,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            max_retries: DEFAULT_MAX_RETRIES,
            max_parallel_readers: DEFAULT_MAX_READERS,
            read_buffer_size: DEFAULT_READ_BUFFER_SIZE,
        }
    }

    pub fn with_auth(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.auth = Some(WebDavAuth {
            username: username.into(),
            password: password.into(),
        });
        self
    }
}

/// WebDAV file metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebDavFileMeta {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    pub etag: Option<String>,
}

/// Metadata cache entry used for incremental scanning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebDavCachedMeta {
    pub meta: WebDavFileMeta,
    pub cached_at: i64,
    pub deleted: bool,
}

/// WebDAV metadata cache backed by sled.
pub struct WebDavMetadataCache {
    db: sled::Db,
    root_key: String,
}

impl WebDavMetadataCache {
    pub fn open(cache_dir: &Path, repo_id: &str) -> Result<Self> {
        let db_path = cache_dir.join(format!("{}.webdav", repo_id));
        std::fs::create_dir_all(cache_dir)?;
        let db = sled::open(&db_path)?;
        Ok(Self {
            db,
            root_key: repo_id.to_string(),
        })
    }

    pub fn get(&self, path: &str) -> Option<WebDavCachedMeta> {
        let key = self.make_key(path);
        self.db
            .get(key)
            .ok()
            .flatten()
            .and_then(|data| bincode::deserialize(&data).ok())
    }

    pub fn put(&self, meta: WebDavCachedMeta) -> Result<()> {
        let key = self.make_key(&meta.meta.path);
        let data = bincode::serialize(&meta)?;
        self.db.insert(key, data)?;
        Ok(())
    }

    pub fn mark_deleted(&self, path: &str) -> Result<()> {
        if let Some(mut cached) = self.get(path) {
            cached.deleted = true;
            cached.cached_at = chrono::Utc::now().timestamp();
            self.put(cached)?;
        }
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = WebDavCachedMeta> + '_ {
        self.db.iter().filter_map(|result| {
            result.ok().and_then(|(_, data)| bincode::deserialize(&data).ok())
        })
    }

    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }

    fn make_key(&self, path: &str) -> Vec<u8> {
        format!("{}:{}", self.root_key, path).into_bytes()
    }
}

/// Result of comparing cached and current metadata.
#[derive(Debug, Clone, Default)]
pub struct MetadataDiff {
    pub unchanged: Vec<WebDavFileMeta>,
    pub changed: Vec<WebDavFileMeta>,
    pub deleted: Vec<String>,
}

impl MetadataDiff {
    pub fn total_changes(&self) -> usize {
        self.changed.len() + self.deleted.len()
    }
}

/// WebDAV scanner response types.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct MultiStatus {
    #[serde(rename = "response")]
    responses: Vec<WebDavResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct WebDavResponse {
    href: String,
    propstat: Vec<WebDavPropStat>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct WebDavPropStat {
    prop: WebDavProp,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct WebDavProp {
    #[serde(rename = "getcontentlength")]
    content_length: Option<String>,
    #[serde(rename = "getlastmodified")]
    last_modified: Option<String>,
    #[serde(rename = "getetag")]
    etag: Option<String>,
    #[serde(rename = "resourcetype")]
    resource_type: Option<WebDavResourceType>,
}

#[derive(Debug, Deserialize)]
struct WebDavResourceType {
    #[serde(rename = "collection")]
    collection: Option<String>,
}

/// WebDAV client that supports discovery and streaming reads.
pub struct WebDavClient {
    config: WebDavConfig,
    client: reqwest::Client,
    semaphore: Arc<Semaphore>,
}

impl WebDavClient {
    pub fn new(config: WebDavConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| BorgError::RemoteConnection(e.to_string()))?;
        Ok(Self {
            semaphore: Arc::new(Semaphore::new(config.max_parallel_readers)),
            config,
            client,
        })
    }

    pub fn config(&self) -> &WebDavConfig {
        &self.config
    }

    pub async fn propfind_recursive(&self) -> Result<Vec<WebDavFileMeta>> {
        let mut stack = VecDeque::new();
        let root = self.normalize_path(&self.config.root_path);
        stack.push_back(root.clone());
        let mut files = Vec::new();

        while let Some(dir) = stack.pop_front() {
            let response = self.propfind(&dir, 1).await?;
            for item in response {
                if item.path == dir {
                    continue;
                }
                if item.is_dir {
                    stack.push_back(item.path.clone());
                } else {
                    files.push(item.meta);
                }
            }
        }

        Ok(files)
    }

    pub fn diff_with_cache(
        &self,
        cache: &WebDavMetadataCache,
        current: &[WebDavFileMeta],
    ) -> MetadataDiff {
        let mut diff = MetadataDiff::default();
        let mut seen: HashSet<String> = HashSet::new();

        for meta in current {
            seen.insert(meta.path.clone());
            if let Some(cached) = cache.get(&meta.path) {
                if cached.deleted {
                    diff.changed.push(meta.clone());
                } else if cached.meta.size == meta.size && cached.meta.mtime == meta.mtime {
                    diff.unchanged.push(meta.clone());
                } else {
                    diff.changed.push(meta.clone());
                }
            } else {
                diff.changed.push(meta.clone());
            }
        }

        for cached in cache.iter() {
            if !seen.contains(&cached.meta.path) && !cached.deleted {
                diff.deleted.push(cached.meta.path);
            }
        }

        diff
    }

    pub fn update_cache(
        &self,
        cache: &WebDavMetadataCache,
        diff: &MetadataDiff,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        for meta in diff.unchanged.iter().chain(diff.changed.iter()) {
            cache.put(WebDavCachedMeta {
                meta: meta.clone(),
                cached_at: now,
                deleted: false,
            })?;
        }
        for deleted in &diff.deleted {
            cache.mark_deleted(deleted)?;
        }
        cache.flush()?;
        Ok(())
    }

    pub async fn stream_reader(&self, path: &str) -> Result<WebDavReader> {
        let normalized = self.normalize_path(path);
        let url = self.resolve_url(&normalized)?;
        Ok(WebDavReader::new(
            self.client.clone(),
            url,
            self.semaphore.clone(),
            self.config.read_buffer_size,
            self.config.max_retries,
        ))
    }

    #[instrument(skip(self))]
    async fn propfind(&self, path: &str, depth: usize) -> Result<Vec<WebDavListingEntry>> {
        let url = self.resolve_url(path)?;
        let mut headers = HeaderMap::new();
        headers.insert("Depth", HeaderValue::from_str(&depth.to_string()).unwrap());
        headers.insert("Content-Type", HeaderValue::from_static("text/xml"));
        let body = r#"<?xml version="1.0"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:resourcetype />
    <d:getcontentlength />
    <d:getlastmodified />
    <d:getetag />
  </d:prop>
</d:propfind>"#;

        let mut request = self.client.request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), url);
        request = request.headers(headers).body(body);
        if let Some(auth) = &self.config.auth {
            request = request.basic_auth(&auth.username, Some(&auth.password));
        }

        let response = request
            .send()
            .await
            .map_err(|e| BorgError::RemoteConnection(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(BorgError::RemoteRepository(format!(
                "PROPFIND failed with status {}",
                status
            )));
        }
        let text = response
            .text()
            .await
            .map_err(|e| BorgError::RemoteConnection(e.to_string()))?;
        self.parse_propfind(&text)
    }

    fn parse_propfind(&self, body: &str) -> Result<Vec<WebDavListingEntry>> {
        let multistatus: MultiStatus = from_str(body)
            .map_err(|e| BorgError::Deserialization(format!("WebDAV XML error: {}", e)))?;
        let mut entries = Vec::new();
        for response in multistatus.responses {
            if let Some(propstat) = response.propstat.into_iter().next() {
                let is_dir = propstat
                    .prop
                    .resource_type
                    .and_then(|rt| rt.collection)
                    .is_some();
                let path = self.normalize_href(&response.href);
                let size = propstat
                    .prop
                    .content_length
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                let mtime = propstat
                    .prop
                    .last_modified
                    .and_then(|s| parse_http_date(&s));
                let etag = propstat.prop.etag.map(|s| s.trim_matches('"').to_string());
                let meta = WebDavFileMeta {
                    path: path.clone(),
                    size,
                    mtime: mtime.unwrap_or(0),
                    etag,
                };
                entries.push(WebDavListingEntry { path, is_dir, meta });
            }
        }
        Ok(entries)
    }

    fn normalize_path(&self, path: &str) -> String {
        let mut normalized = path.trim().to_string();
        if !normalized.starts_with('/') {
            normalized.insert(0, '/');
        }
        if normalized.ends_with('/') && normalized.len() > 1 {
            normalized.pop();
        }
        normalized
    }

    fn normalize_href(&self, href: &str) -> String {
        let path = href.split('?').next().unwrap_or(href);
        let mut decoded = percent_decode(path);
        decoded = decoded.trim_end_matches('/').to_string();
        if decoded.is_empty() {
            decoded = "/".to_string();
        }
        decoded
    }

    fn resolve_url(&self, path: &str) -> Result<Url> {
        let normalized = self.normalize_path(path);
        let base = self.config.base_url.clone();
        let joined = base
            .join(&normalized)
            .map_err(|e| BorgError::InvalidArgument(format!("Invalid URL: {}", e)))?;
        Ok(joined)
    }
}

/// Parsed WebDAV listing entry.
#[derive(Debug, Clone)]
struct WebDavListingEntry {
    path: String,
    is_dir: bool,
    meta: WebDavFileMeta,
}

/// Streaming WebDAV reader with range support.
pub struct WebDavReader {
    client: reqwest::Client,
    url: Url,
    semaphore: Arc<Semaphore>,
    buffer_size: usize,
    max_retries: usize,
    accept_ranges: Option<bool>,
    etag: Option<String>,
    size: Option<u64>,
}

impl WebDavReader {
    fn new(
        client: reqwest::Client,
        url: Url,
        semaphore: Arc<Semaphore>,
        buffer_size: usize,
        max_retries: usize,
    ) -> Self {
        Self {
            client,
            url,
            semaphore,
            buffer_size,
            max_retries,
            accept_ranges: None,
            etag: None,
            size: None,
        }
    }

    pub async fn open(&mut self) -> Result<()> {
        let response = self
            .client
            .head(self.url.clone())
            .send()
            .await
            .map_err(|e| BorgError::RemoteConnection(e.to_string()))?;
        if !response.status().is_success() {
            return Err(BorgError::RemoteRepository(format!(
                "HEAD request failed: {}",
                response.status()
            )));
        }

        self.accept_ranges = response
            .headers()
            .get(reqwest::header::ACCEPT_RANGES)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.contains("bytes"));
        self.etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim_matches('"').to_string());
        self.size = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        Ok(())
    }

    pub fn supports_ranges(&self) -> bool {
        self.accept_ranges.unwrap_or(false)
    }

    pub fn size(&self) -> Option<u64> {
        self.size
    }

    pub fn etag(&self) -> Option<&str> {
        self.etag.as_deref()
    }

    pub async fn stream_from(&self, offset: u64) -> Result<WebDavStream> {
        if offset > 0 && !self.supports_ranges() {
            return Err(BorgError::RemoteRepository(
                "Server does not support range requests".to_string(),
            ));
        }

        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| BorgError::Cancelled)?;
        let mut request = self.client.get(self.url.clone());

        if offset > 0 {
            request = request.header(RANGE, format!("bytes={}-", offset));
            if let Some(etag) = &self.etag {
                request = request.header(IF_RANGE, etag.clone());
            }
        }

        let response = request
            .send()
            .await
            .map_err(|e| BorgError::RemoteConnection(e.to_string()))?;

        if offset > 0 && response.status() == reqwest::StatusCode::OK {
            return Err(BorgError::RemoteRepository(
                "Server ignored range request".to_string(),
            ));
        }

        if offset > 0 {
            if let Some(range) = response.headers().get(reqwest::header::CONTENT_RANGE) {
                let range = range.to_str().unwrap_or_default();
                if !range.starts_with(&format!("bytes {}-", offset)) {
                    return Err(BorgError::IntegrityCheck {
                        expected: format!("range starting at {}", offset),
                        actual: range.to_string(),
                    });
                }
            }
        }

        if !response.status().is_success() {
            return Err(BorgError::RemoteRepository(format!(
                "GET failed with status {}",
                response.status()
            )));
        }

        let stream = response.bytes_stream();
        Ok(WebDavStream::new(stream, self.buffer_size, permit))
    }

    pub async fn stream_with_retry(&self, offset: u64) -> Result<WebDavStream> {
        let mut attempt = 0;
        loop {
            match self.stream_from(offset).await {
                Ok(stream) => return Ok(stream),
                Err(err) => {
                    attempt += 1;
                    if attempt > self.max_retries {
                        return Err(err);
                    }
                    warn!("Retrying WebDAV stream after error: {}", err);
                    tokio::time::sleep(Duration::from_millis(200 * attempt as u64)).await;
                }
            }
        }
    }
}

/// Stream wrapper for WebDAV responses.
type WebDavBytesStream = std::pin::Pin<
    Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + Sync>,
>;

pub struct WebDavStream {
    reader: StreamReader<WebDavBytesStream, Bytes>,

    _permit: OwnedSemaphorePermit,
}

impl WebDavStream {
    fn new(
        stream: impl Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send + Sync + 'static,
        _buffer_size: usize,
        permit: OwnedSemaphorePermit,
    ) -> Self {
        let stream = stream.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
        let stream: WebDavBytesStream = Box::pin(stream);
        let reader = StreamReader::new(stream);
        Self {
            reader,
            _permit: permit,
        }
    }

    pub fn into_reader(self) -> impl tokio::io::AsyncRead {
        self.reader
    }
}

/// Descriptor for a chunk in a streamed file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkDescriptor {
    pub id: Option<ChunkId>,
    pub offset: u64,
    pub size: usize,
}

/// Stream a WebDAV reader into content-defined chunks.
pub async fn stream_to_chunks<R>(
    mut reader: R,
    chunker: &Chunker,
    chunk_callback: &mut impl FnMut(ChunkDescriptor, Vec<u8>) -> Result<()>,
) -> Result<Vec<ChunkDescriptor>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buffer = vec![0u8; chunker.config().max_size as usize * 2];
    let mut pending: Vec<u8> = Vec::new();
    let mut descriptors = Vec::new();
    let mut offset: u64 = 0;

    loop {
        let bytes_read = tokio::io::AsyncReadExt::read(&mut reader, &mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        pending.extend_from_slice(&buffer[..bytes_read]);

        if pending.len() >= chunker.config().max_size as usize {
            let chunks = chunker.chunk_data(&pending);
            if chunks.len() > 1 {
                let mut processed = 0usize;
                let count = chunks.len();
                for chunk in chunks.into_iter().take(count - 1) {
                    let descriptor = ChunkDescriptor {
                        id: Some(chunk.id.clone()),
                        offset,
                        size: chunk.original_size,
                    };
                    offset += chunk.original_size as u64;
                    processed += chunk.original_size;
                    chunk_callback(descriptor.clone(), chunk.data)?;
                    descriptors.push(descriptor);
                }
                pending = pending.split_off(processed);
            }
        }
    }

    if !pending.is_empty() {
        for chunk in chunker.chunk_data(&pending) {
            let descriptor = ChunkDescriptor {
                id: Some(chunk.id.clone()),
                offset,
                size: chunk.original_size,
            };
            offset += chunk.original_size as u64;
            chunk_callback(descriptor.clone(), chunk.data)?;
            descriptors.push(descriptor);
        }
    }

    Ok(descriptors)
}

fn parse_http_date(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc2822(value)
        .map(|dt| dt.timestamp())
        .ok()
}

fn percent_decode(value: &str) -> String {
    let mut result = String::new();
    let mut i = 0;
    let bytes = value.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    result.push(byte as char);
                    i += 3;
                    continue;
                }
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

/// Chunking configuration for WebDAV streams.
#[derive(Debug, Clone)]
pub struct WebDavChunkingConfig {
    pub chunker_config: ChunkerConfig,
}

impl Default for WebDavChunkingConfig {
    fn default() -> Self {
        Self {
            chunker_config: ChunkerConfig::default(),
        }
    }
}

/// High-level pipeline for WebDAV streaming chunking.
pub struct WebDavChunkPipeline {
    chunker: Chunker,
}

impl WebDavChunkPipeline {
    pub fn new(config: WebDavChunkingConfig) -> Result<Self> {
        Ok(Self {
            chunker: Chunker::new(config.chunker_config)?,
        })
    }

    pub fn chunker(&self) -> &Chunker {
        &self.chunker
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::MockServer;
    use tokio::io::AsyncReadExt;

    #[test]
    fn diff_detects_changes_and_deletes() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache = WebDavMetadataCache::open(dir.path(), "repo").unwrap();
        let meta = WebDavFileMeta {
            path: "/data/a.txt".to_string(),
            size: 10,
            mtime: 100,
            etag: None,
        };
        cache
            .put(WebDavCachedMeta {
                meta: meta.clone(),
                cached_at: 0,
                deleted: false,
            })
            .unwrap();

        let client = WebDavClient::new(WebDavConfig::new(
            Url::parse("http://localhost/").unwrap(),
            "/".to_string(),
        ))
        .unwrap();
        let updated = WebDavFileMeta {
            path: "/data/a.txt".to_string(),
            size: 11,
            mtime: 101,
            etag: None,
        };
        let diff = client.diff_with_cache(&cache, &[updated.clone()]);
        assert_eq!(diff.changed.len(), 1);
        assert!(diff.deleted.is_empty());

        let diff = client.diff_with_cache(&cache, &[]);
        assert_eq!(diff.deleted, vec!["/data/a.txt".to_string()]);
    }

    #[tokio::test]
    async fn propfind_parsing() {
        let server = MockServer::start();
        let body = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/root/</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype><d:collection/></d:resourcetype>
      </d:prop>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/root/file.txt</d:href>
    <d:propstat>
      <d:prop>
        <d:getcontentlength>12</d:getcontentlength>
        <d:getlastmodified>Wed, 21 Oct 2015 07:28:00 GMT</d:getlastmodified>
        <d:getetag>"\"etag\""</d:getetag>
      </d:prop>
    </d:propstat>
  </d:response>
</d:multistatus>"#;

        let _mock = server.mock(|when, then| {
            when.path_contains("/root");
            then.status(207).body(body);
        });

        let config = WebDavConfig::new(
            Url::parse(&server.base_url()).unwrap(),
            "/root".to_string(),
        );
        let client = WebDavClient::new(config).unwrap();
        let entries = client.propfind_recursive().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "/root/file.txt");
        assert_eq!(entries[0].size, 12);
        assert_eq!(entries[0].mtime, 1445412480);
    }

    #[tokio::test]
    async fn stream_to_chunks_produces_descriptors() {
        let data = vec![1u8; 256 * 1024];
        let mut reader: &[u8] = &data;
        let chunker = Chunker::with_defaults();
        let mut seen = Vec::new();
        let descriptors = stream_to_chunks(&mut reader, &chunker, &mut |descriptor, _| {
            seen.push(descriptor.clone());
            Ok(())
        })
        .await
        .unwrap();

        assert!(!descriptors.is_empty());
        assert_eq!(descriptors.len(), seen.len());
        assert_eq!(descriptors[0].offset, 0);
    }

    #[tokio::test]
    async fn range_request_math() {
        let server = MockServer::start();
        let data = vec![0u8; 64];
        let _head = server.mock(|when, then| {
            when.method(httpmock::Method::HEAD).path("/file.bin");
            then.status(200)
                .header("Accept-Ranges", "bytes")
                .header("Content-Length", "64")
                .header("ETag", "\"abc\"");
        });
        let _get = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/file.bin")
                .header_exists("Range");
            then.status(206).body(data.clone());
        });

        let config = WebDavConfig::new(
            Url::parse(&server.base_url()).unwrap(),
            "/".to_string(),
        );
        let client = WebDavClient::new(config).unwrap();
        let mut reader = client.stream_reader("/file.bin").await.unwrap();
        reader.open().await.unwrap();
        assert!(reader.supports_ranges());
        let stream = reader.stream_from(10).await.unwrap();
        let mut async_reader = stream.into_reader();
        let mut out = Vec::new();
        async_reader.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, data);
    }
}