//! FCast v4 sender boundary.

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use thiserror::Error;
use url::Url;

#[cfg(feature = "upstream")]
pub mod upstream;

pub const FCAST_V4: u8 = 4;

#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum FcastError {
    #[error("receiver endpoint is invalid: {0}")]
    InvalidEndpoint(String),
    #[error("media URL is invalid: {0}")]
    InvalidMediaUrl(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiverEndpoint {
    pub id: String,
    pub name: String,
    pub host: IpAddr,
    pub port: u16,
    pub fingerprint: String,
}

impl ReceiverEndpoint {
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }

    pub fn validate(&self) -> Result<(), FcastError> {
        if self.port == 0 || self.id.is_empty() || self.fingerprint.is_empty() {
            return Err(FcastError::InvalidEndpoint(
                "missing id, port, or fingerprint".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadRequest {
    pub url: String,
    pub title: Option<String>,
    pub content_type: Option<String>,
}

impl LoadRequest {
    pub fn validate(&self) -> Result<Url, FcastError> {
        let url = Url::parse(&self.url)
            .map_err(|error| FcastError::InvalidMediaUrl(error.to_string()))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(FcastError::InvalidMediaUrl(
                "only HTTP(S) URLs are supported".into(),
            ));
        }
        if url.username() != "" || url.password().is_some() {
            return Err(FcastError::InvalidMediaUrl(
                "embedded credentials are not allowed".into(),
            ));
        }
        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_rejects_credentials_and_non_http() {
        let credentials = LoadRequest {
            url: "https://user:password@example.com/video".into(),
            title: None,
            content_type: None,
        };
        assert!(credentials.validate().is_err());
        let file = LoadRequest {
            url: "file:///tmp/video".into(),
            title: None,
            content_type: None,
        };
        assert!(file.validate().is_err());
    }
}
