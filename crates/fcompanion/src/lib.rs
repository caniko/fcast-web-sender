//! In-memory FCompanion resource registry.
//!
//! FCompanion is intentionally a local HTTP resource channel. The native
//! bridge carries opaque resource metadata and tokens, while media bytes stay
//! on the local HTTP connection to the receiver.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use url::Url;

const DEFAULT_TTL: Duration = Duration::from_secs(15 * 60);
const DEFAULT_MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum CompanionError {
    #[error("resource is {actual} bytes; maximum is {maximum}")]
    TooLarge { actual: usize, maximum: usize },
    #[error("resource content type is empty or invalid")]
    InvalidContentType,
    #[error("resource token was not found or expired")]
    NotFound,
    #[error("companion URL is not a loopback HTTP URL")]
    InvalidBaseUrl,
    #[error("companion HTTP I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct Resource {
    pub token: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub expires_at: SystemTime,
}

#[derive(Debug)]
pub struct ResourceStore {
    base_url: Url,
    ttl: Duration,
    max_bytes: usize,
    resources: BTreeMap<String, Resource>,
    nonce: u64,
}

impl ResourceStore {
    pub fn new(base_url: Url) -> Result<Self, CompanionError> {
        validate_base_url(&base_url)?;
        Ok(Self {
            base_url,
            ttl: DEFAULT_TTL,
            max_bytes: DEFAULT_MAX_BYTES,
            resources: BTreeMap::new(),
            nonce: 0,
        })
    }

    pub fn with_limits(
        base_url: Url,
        ttl: Duration,
        max_bytes: usize,
    ) -> Result<Self, CompanionError> {
        validate_base_url(&base_url)?;
        Ok(Self {
            base_url,
            ttl,
            max_bytes,
            resources: BTreeMap::new(),
            nonce: 0,
        })
    }

    pub fn register(
        &mut self,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<String, CompanionError> {
        if bytes.len() > self.max_bytes {
            return Err(CompanionError::TooLarge {
                actual: bytes.len(),
                maximum: self.max_bytes,
            });
        }
        if content_type.is_empty()
            || !content_type
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || byte == b' ')
            || content_type.contains('\r')
            || content_type.contains('\n')
        {
            return Err(CompanionError::InvalidContentType);
        }
        self.nonce = self.nonce.wrapping_add(1);
        let mut hasher = Sha256::new();
        hasher.update(std::process::id().to_be_bytes());
        hasher.update(self.nonce.to_be_bytes());
        hasher.update(now_millis().to_be_bytes());
        hasher.update(&bytes[..bytes.len().min(64)]);
        let token = hex::encode(hasher.finalize());
        self.resources.insert(
            token.clone(),
            Resource {
                token: token.clone(),
                content_type: content_type.into(),
                bytes,
                expires_at: SystemTime::now() + self.ttl,
            },
        );
        Ok(token)
    }

    pub fn get(&mut self, token: &str) -> Result<&Resource, CompanionError> {
        self.expire();
        self.resources.get(token).ok_or(CompanionError::NotFound)
    }

    pub fn remove(&mut self, token: &str) -> bool {
        self.resources.remove(token).is_some()
    }

    pub fn expire(&mut self) {
        let now = SystemTime::now();
        self.resources
            .retain(|_, resource| resource.expires_at > now);
    }

    pub fn resource_url(&self, token: &str) -> Result<Url, CompanionError> {
        if !self.resources.contains_key(token) {
            return Err(CompanionError::NotFound);
        }
        let mut url = self.base_url.clone();
        let path = format!("resource/{token}");
        url.set_path(&path);
        Ok(url)
    }

    pub fn len(&self) -> usize {
        self.resources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }
}

#[derive(Debug)]
pub struct CompanionHttpServer {
    listener: TcpListener,
    store: Arc<Mutex<ResourceStore>>,
}

impl CompanionHttpServer {
    pub async fn bind(ttl: Duration, max_bytes: usize) -> Result<Self, CompanionError> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let base_url = Url::parse(&format!("http://127.0.0.1:{port}"))
            .map_err(|_| CompanionError::InvalidBaseUrl)?;
        let store = ResourceStore::with_limits(base_url, ttl, max_bytes)?;
        Ok(Self {
            listener,
            store: Arc::new(Mutex::new(store)),
        })
    }

    pub fn local_addr(&self) -> Result<std::net::SocketAddr, CompanionError> {
        Ok(self.listener.local_addr()?)
    }

    pub fn store(&self) -> Arc<Mutex<ResourceStore>> {
        Arc::clone(&self.store)
    }

    pub async fn serve_next(&self) -> Result<(), CompanionError> {
        let (stream, _) = self.listener.accept().await?;
        serve_connection(stream, Arc::clone(&self.store)).await
    }
}

async fn serve_connection(
    mut stream: TcpStream,
    store: Arc<Mutex<ResourceStore>>,
) -> Result<(), CompanionError> {
    let mut request = vec![0_u8; 8 * 1024];
    let mut length = 0;
    loop {
        if length == request.len() {
            write_response(&mut stream, 431, "text/plain", b"request too large\n").await?;
            return Ok(());
        }
        let read = stream.read(&mut request[length..]).await?;
        if read == 0 {
            return Ok(());
        }
        length += read;
        if request[..length]
            .windows(4)
            .any(|window| window == b"\r\n\r\n")
        {
            break;
        }
    }
    let header = String::from_utf8_lossy(&request[..length]);
    let Some(first_line) = header.lines().next() else {
        write_response(&mut stream, 400, "text/plain", b"bad request\n").await?;
        return Ok(());
    };
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    if method != "GET" {
        write_response(&mut stream, 405, "text/plain", b"method not allowed\n").await?;
        return Ok(());
    }
    let Some(token) = path.strip_prefix("/resource/") else {
        write_response(&mut stream, 404, "text/plain", b"not found\n").await?;
        return Ok(());
    };
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        write_response(&mut stream, 404, "text/plain", b"not found\n").await?;
        return Ok(());
    }
    let range = header
        .lines()
        .find_map(|line| {
            line.strip_prefix("Range:")
                .or_else(|| line.strip_prefix("range:"))
        })
        .and_then(parse_range);
    let resource = store.lock().await.get(token).ok().cloned();
    match resource {
        Some(resource) => match range {
            Some((start, end)) if start < resource.bytes.len() => {
                let end = end.min(resource.bytes.len().saturating_sub(1));
                write_partial_response(
                    &mut stream,
                    &resource.content_type,
                    &resource.bytes[start..=end],
                    start,
                    end,
                    resource.bytes.len(),
                )
                .await?
            }
            Some(_) => {
                write_response(&mut stream, 416, "text/plain", b"range not satisfiable\n").await?
            }
            None => {
                write_response(&mut stream, 200, &resource.content_type, &resource.bytes).await?
            }
        },
        None => write_response(&mut stream, 404, "text/plain", b"not found\n").await?,
    }
    Ok(())
}

fn parse_range(raw: &str) -> Option<(usize, usize)> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("bytes=")?;
    if raw.contains(',') {
        return None;
    }
    let (start, end) = raw.split_once('-')?;
    let start = start.parse().ok()?;
    let end = if end.trim().is_empty() {
        usize::MAX
    } else {
        end.parse().ok()?
    };
    (start <= end).then_some((start, end))
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), CompanionError> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        431 => "Request Header Fields Too Large",
        416 => "Range Not Satisfiable",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn write_partial_response(
    stream: &mut TcpStream,
    content_type: &str,
    body: &[u8],
    start: usize,
    end: usize,
    total: usize,
) -> Result<(), CompanionError> {
    let header = format!(
        "HTTP/1.1 206 Partial Content\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{total}\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await?;
    Ok(())
}

fn validate_base_url(url: &Url) -> Result<(), CompanionError> {
    if url.scheme() != "http"
        || !matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
        )
    {
        return Err(CompanionError::InvalidBaseUrl);
    }
    Ok(())
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_are_opaque_and_local() {
        let base = Url::parse("http://127.0.0.1:43781").unwrap();
        let mut store = ResourceStore::with_limits(base, Duration::from_secs(60), 128).unwrap();
        let token = store.register("video/mp4", vec![1, 2, 3]).unwrap();
        assert_ne!(token, "video.mp4");
        assert_eq!(store.resource_url(&token).unwrap().scheme(), "http");
        assert_eq!(store.get(&token).unwrap().bytes, vec![1, 2, 3]);
    }

    #[test]
    fn non_loopback_base_is_rejected() {
        let error = ResourceStore::new(Url::parse("http://192.0.2.10:80").unwrap()).unwrap_err();
        assert!(matches!(error, CompanionError::InvalidBaseUrl));
    }

    #[tokio::test]
    async fn local_http_server_serves_registered_bytes_only_by_token() {
        let server = CompanionHttpServer::bind(Duration::from_secs(60), 128)
            .await
            .unwrap();
        let store = server.store();
        let token = store
            .lock()
            .await
            .register("video/mp4", vec![1, 2, 3])
            .unwrap();
        let address = server.local_addr().unwrap();
        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream
                .write_all(
                    format!("GET /resource/{token} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
                )
                .await
                .unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).await.unwrap();
            response
        });
        server.serve_next().await.unwrap();
        let response = client.await.unwrap();
        assert!(response.windows(3).any(|window| window == [1, 2, 3]));
    }

    #[tokio::test]
    async fn local_http_server_honors_single_byte_ranges() {
        let server = CompanionHttpServer::bind(Duration::from_secs(60), 128)
            .await
            .unwrap();
        let token = server
            .store()
            .lock()
            .await
            .register("video/mp4", b"abcdef".to_vec())
            .unwrap();
        let address = server.local_addr().unwrap();
        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream
                .write_all(
                    format!(
                        "GET /resource/{token} HTTP/1.1\r\nHost: localhost\r\nRange: bytes=2-4\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).await.unwrap();
            response
        });
        server.serve_next().await.unwrap();
        let response = client.await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 206"));
        assert!(response.ends_with(b"cde"));
    }
}
