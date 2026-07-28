//! Bounded manifest fetching with an explicit SSRF policy.

use fcast_manifest_rewriter::{RewriteError, rewrite_dash, rewrite_hls};
use reqwest::Client;
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use url::Url;

const DEFAULT_MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MAX_RESOURCE_BYTES: usize = 64 * 1024 * 1024;
static FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("media URL rejected by policy: {0}")]
    RejectedUrl(String),
    #[error("manifest response was redirected")]
    Redirect,
    #[error("manifest response is {actual} bytes; maximum is {maximum}")]
    TooLarge { actual: u64, maximum: usize },
    #[error("manifest HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("media hostname could not be resolved safely: {0}")]
    Dns(String),
    #[error("manifest returned HTTP status {0}")]
    Status(reqwest::StatusCode),
    #[error("manifest rewrite failed: {0}")]
    Rewrite(#[from] RewriteError),
    #[error("media header is invalid: {0}")]
    InvalidHeader(String),
    #[error("media resource file I/O failed: {0}")]
    FileIo(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy)]
pub struct UrlPolicy {
    pub allow_private_ip_literals: bool,
    pub max_manifest_bytes: usize,
    pub max_resource_bytes: usize,
}

impl Default for UrlPolicy {
    fn default() -> Self {
        Self {
            allow_private_ip_literals: false,
            max_manifest_bytes: DEFAULT_MAX_MANIFEST_BYTES,
            max_resource_bytes: DEFAULT_MAX_RESOURCE_BYTES,
        }
    }
}

impl UrlPolicy {
    pub fn validate(&self, url: &Url) -> Result<(), FetchError> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(FetchError::RejectedUrl(
                "only HTTP(S) media URLs are supported".into(),
            ));
        }
        if url.username() != "" || url.password().is_some() {
            return Err(FetchError::RejectedUrl(
                "URLs with embedded credentials are not allowed".into(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| FetchError::RejectedUrl("media URL has no host".into()))?;
        if !self.allow_private_ip_literals {
            if let Ok(address) = host.parse::<IpAddr>()
                && is_private_or_local(address)
            {
                return Err(FetchError::RejectedUrl("private or local address".into()));
            }
            let lower = host.to_ascii_lowercase();
            if lower == "localhost" || lower.ends_with(".local") {
                return Err(FetchError::RejectedUrl("local hostname".into()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct MediaFetcher {
    policy: UrlPolicy,
}

impl MediaFetcher {
    pub fn new(policy: UrlPolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> UrlPolicy {
        self.policy
    }

    pub async fn fetch_manifest(&self, url: &Url) -> Result<(String, String), FetchError> {
        let client = self.client_for_url(url).await?;
        let response = client.get(url.clone()).send().await?;
        if response.status().is_redirection() {
            return Err(FetchError::Redirect);
        }
        if !response.status().is_success() {
            return Err(FetchError::Status(response.status()));
        }
        if let Some(length) = response.content_length()
            && length > self.policy.max_manifest_bytes as u64
        {
            return Err(FetchError::TooLarge {
                actual: length,
                maximum: self.policy.max_manifest_bytes,
            });
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| response_content_type(url));
        let bytes = response.bytes().await?;
        if bytes.len() > self.policy.max_manifest_bytes {
            return Err(FetchError::TooLarge {
                actual: bytes.len() as u64,
                maximum: self.policy.max_manifest_bytes,
            });
        }
        let body = String::from_utf8_lossy(&bytes).into_owned();
        Ok((content_type, body))
    }

    pub async fn fetch_to_file(
        &self,
        url: &Url,
        headers: &[(String, String)],
        allow_sensitive_headers: bool,
    ) -> Result<FetchedFile, FetchError> {
        let client = self.client_for_url(url).await?;
        let mut request = client.get(url.clone());
        for (name, value) in headers {
            let lower = name.to_ascii_lowercase();
            let sensitive = matches!(lower.as_str(), "authorization" | "cookie");
            if sensitive && !allow_sensitive_headers {
                return Err(FetchError::InvalidHeader(format!(
                    "sensitive header '{name}' requires an explicit credential lease"
                )));
            }
            if !sensitive
                && !matches!(
                    lower.as_str(),
                    "accept" | "accept-language" | "origin" | "referer" | "user-agent"
                )
            {
                return Err(FetchError::InvalidHeader(format!(
                    "header '{name}' is not in the media allowlist"
                )));
            }
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| FetchError::InvalidHeader(error.to_string()))?;
            let header_value = HeaderValue::from_str(value)
                .map_err(|error| FetchError::InvalidHeader(error.to_string()))?;
            request = request.header(header_name, header_value);
        }
        let response = request.send().await?;
        if response.status().is_redirection() {
            return Err(FetchError::Redirect);
        }
        if !response.status().is_success() {
            return Err(FetchError::Status(response.status()));
        }
        if let Some(length) = response.content_length()
            && length > self.policy.max_resource_bytes as u64
        {
            return Err(FetchError::TooLarge {
                actual: length,
                maximum: self.policy.max_resource_bytes,
            });
        }

        let path = temporary_path().await?;
        let mut file = match tokio::fs::File::create(&path).await {
            Ok(file) => file,
            Err(error) => {
                let _ = tokio::fs::remove_file(&path).await;
                return Err(error.into());
            }
        };
        let mut length = 0_u64;
        let result = async {
            let mut response = response;
            while let Some(chunk) = response.chunk().await? {
                length = length.saturating_add(chunk.len() as u64);
                if length > self.policy.max_resource_bytes as u64 {
                    return Err(FetchError::TooLarge {
                        actual: length,
                        maximum: self.policy.max_resource_bytes,
                    });
                }
                file.write_all(&chunk).await?;
            }
            file.flush().await?;
            Ok::<(), FetchError>(())
        }
        .await;
        if let Err(error) = result {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(error);
        }
        let content_type = response_content_type(url);
        Ok(FetchedFile {
            path,
            content_type,
            length,
        })
    }

    pub fn rewrite_manifest<F>(
        &self,
        content_type: &str,
        source: &Url,
        body: &str,
        rewrite_url: F,
    ) -> Result<String, FetchError>
    where
        F: FnMut(&Url) -> String,
    {
        if content_type.contains("dash") || source.path().ends_with(".mpd") {
            Ok(rewrite_dash(body, source, rewrite_url)?)
        } else {
            Ok(rewrite_hls(body, source, rewrite_url)?)
        }
    }

    async fn client_for_url(&self, url: &Url) -> Result<Client, FetchError> {
        self.policy.validate(url)?;
        let host = url
            .host_str()
            .ok_or_else(|| FetchError::RejectedUrl("media URL has no host".into()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| FetchError::RejectedUrl("media URL has no known port".into()))?;
        let mut addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| FetchError::Dns(error.to_string()))?;
        let address = addresses
            .find(|address| {
                self.policy.allow_private_ip_literals || !is_private_or_local(address.ip())
            })
            .ok_or_else(|| {
                FetchError::RejectedUrl(
                    "hostname resolved only to private or local addresses".into(),
                )
            })?;
        Client::builder()
            .redirect(Policy::none())
            .user_agent("fcast-web-sender/0.1")
            .resolve(host, SocketAddr::new(address.ip(), port))
            .build()
            .map_err(FetchError::Http)
    }
}

#[derive(Debug)]
pub struct FetchedFile {
    path: PathBuf,
    pub content_type: String,
    pub length: u64,
}

impl FetchedFile {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for FetchedFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn temporary_path() -> Result<PathBuf, std::io::Error> {
    let directory = std::env::temp_dir().join("fcast-web-sender");
    tokio::fs::create_dir_all(&directory).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).await?;
    }
    loop {
        let counter = FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!("{}-{counter}.media", std::process::id()));
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        match options.open(&path).await {
            Ok(file) => {
                drop(file);
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

fn response_content_type(url: &Url) -> String {
    if url.path().ends_with(".mpd") {
        "application/dash+xml".into()
    } else {
        "application/vnd.apple.mpegurl".into()
    }
}

fn is_private_or_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_broadcast()
                || address.is_multicast()
                || address.octets()[0] == 100 && (64..=127).contains(&address.octets()[1])
                || address.octets()[0] == 192
                    && address.octets()[1] == 0
                    && address.octets()[2] == 0
        }
        IpAddr::V6(address) => {
            address.is_loopback()
                || address.is_unspecified()
                || address.is_multicast()
                || is_unique_local_v6(address)
                || is_link_local_v6(address)
        }
    }
}

fn is_unique_local_v6(address: Ipv6Addr) -> bool {
    (address.segments()[0] & 0xfe00) == 0xfc00
}

fn is_link_local_v6(address: Ipv6Addr) -> bool {
    (address.segments()[0] & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_rejects_private_literal_and_credentials() {
        let policy = UrlPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://127.0.0.1/video.m3u8").unwrap())
                .is_err()
        );
        assert!(
            policy
                .validate(&Url::parse("https://user:pass@example.com/video.m3u8").unwrap())
                .is_err()
        );
        assert!(
            policy
                .validate(&Url::parse("https://media.example/video.m3u8").unwrap())
                .is_ok()
        );
    }

    #[test]
    fn manifest_type_is_selected_without_network_access() {
        let fetcher = MediaFetcher::new(UrlPolicy::default());
        let source = Url::parse("https://media.example/master.m3u8").unwrap();
        let output = fetcher
            .rewrite_manifest(
                "application/vnd.apple.mpegurl",
                &source,
                "#EXTM3U\nsegment.ts\n",
                |url| format!("http://127.0.0.1/resource/{}", url.path().trim_matches('/')),
            )
            .unwrap();
        assert!(output.contains("resource/segment.ts"));
    }

    #[tokio::test]
    async fn fetched_file_is_removed_when_the_lease_ends() {
        let path = temporary_path().await.unwrap();
        tokio::fs::write(&path, b"fixture").await.unwrap();
        let lease = FetchedFile {
            path: path.clone(),
            content_type: "video/mp4".into(),
            length: 7,
        };
        assert!(lease.path().exists());
        drop(lease);
        assert!(!path.exists());
    }
}
