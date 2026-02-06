# WebDAV Ingestion Pipeline

This document describes the WebDAV ingestion components implemented in `borg-core::webdav`.
They provide discovery, metadata caching for incremental scans, streaming readers with
range/resume support, and content-defined chunking for deduplication.

## 1. WebDAV Discovery & Metadata Cache

### Recursive PROPFIND Scanner

`WebDavClient::propfind_recursive` performs a breadth-first walk of a WebDAV tree:

1. Issue a `PROPFIND` request with `Depth: 1`.
2. Parse the XML multi-status response into typed structs.
3. Normalize hrefs to path strings (no trailing slashes, leading `/`).
4. Enqueue directories; emit files as `WebDavFileMeta`.

Each file metadata object includes:

- `path`: normalized path under the WebDAV root
- `size`: from `getcontentlength`
- `mtime`: parsed RFC2822 `getlastmodified`
- `etag`: optional ETag (if present)

### Metadata Cache (Incremental Scans)

`WebDavMetadataCache` stores `WebDavCachedMeta` entries in a sled database:

- Key: `"<repo_id>:<path>"`
- Values are bincode-serialized `WebDavCachedMeta`

`WebDavClient::diff_with_cache` compares a fresh scan to cached entries and returns:

- `unchanged`: size + mtime match
- `changed`: new or modified entries
- `deleted`: cached entries no longer present

`WebDavClient::update_cache` persists the diff and marks deleted entries so future
scans can track removals without losing history.

## 2. Streaming WebDAV Reader (Range/Resume)

`WebDavReader` is constructed via `WebDavClient::stream_reader` and supports:

- `open()` for a HEAD request to determine size, ETag, and range support.
- `stream_from(offset)` to issue a ranged GET request and validate `Content-Range`.
- `stream_with_retry(offset)` to retry transient failures with exponential backoff.

Concurrency is bounded by a semaphore shared across readers so only a configured
number of parallel streams are active at once.

If a server does not advertise `Accept-Ranges: bytes`, the reader rejects resume
requests and requires a full restart.

## 3. Content-Defined Chunking Pipeline

`stream_to_chunks` accepts an async reader and uses the existing FastCDC-based
`Chunker` to produce deterministic chunk boundaries while keeping memory usage
bounded:

- Reads a fixed buffer into `pending` memory.
- Once `pending` reaches `max_size`, runs FastCDC.
- Emits all but the last chunk (kept to allow boundary overlap).

Returned `ChunkDescriptor` values capture the streaming offsets for each chunk, which
can later be used for archive metadata or resuming uploads.

`WebDavChunkPipeline` provides a higher-level API to configure the chunker.

## 4. Testing Strategy

- Mocked WebDAV responses are tested using `httpmock`.
- Metadata cache diff logic is validated in unit tests.
- Range request handling is validated with HEAD + GET mocks.
- Chunking logic is verified through a streaming pipeline test.

## 5. Notes & Constraints

- Paths are normalized (leading `/`, no trailing `/`).
- Only size + mtime are used to detect unchanged files (etag retained for later).
- Range integrity is checked via `Content-Range` when resuming.
- All streaming operations use bounded buffers to prevent unbounded memory growth.


## 6. Example usage

# Initialize a new repo on a WebDAV server
borg init webdavs://alice:secret@backup.example.com/borg-repo

# List archives later
borg --repo webdavs://alice:secret@backup.example.com/borg-repo list