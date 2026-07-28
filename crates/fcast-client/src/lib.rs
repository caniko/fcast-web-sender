//! FCast v4 transport boundary.
//!
//! The browser-facing bridge uses this crate's typed commands and state
//! machine. The wire adapter can later attach the upstream FCast sender SDK
//! without leaking protocol details into the extension or companion router.
//! Version negotiation is intentionally strict: v3/legacy fallback is not
//! enabled by this project.

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use thiserror::Error;
use url::Url;

#[cfg(feature = "upstream")]
pub mod upstream;

pub const FCAST_V4: u8 = 4;
pub const MAX_PACKET_BYTES: usize = 512 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FcastError {
    #[error("only FCast v4 is supported")]
    UnsupportedVersion,
    #[error("receiver is not explicitly trusted")]
    ReceiverNotTrusted,
    #[error("receiver endpoint is invalid: {0}")]
    InvalidEndpoint(String),
    #[error("media URL is invalid: {0}")]
    InvalidMediaUrl(String),
    #[error("FCast session is not connected")]
    NotConnected,
    #[error("FCast packet is {actual} bytes; maximum is {maximum}")]
    PacketTooLarge { actual: usize, maximum: usize },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FcastVersion {
    V4,
}

pub fn negotiate_version(peer_versions: &[u8]) -> Result<FcastVersion, FcastError> {
    if peer_versions.contains(&FCAST_V4) {
        Ok(FcastVersion::V4)
    } else {
        Err(FcastError::UnsupportedVersion)
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FcastCommand {
    Load { request: LoadRequest },
    Play,
    Pause,
    Stop,
    Seek { position_ms: u64 },
    SetVolume { level: f32 },
    SelectTrack { track_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Debug)]
pub struct FcastSession {
    endpoint: ReceiverEndpoint,
    state: ConnectionState,
    version: Option<FcastVersion>,
}

impl FcastSession {
    pub fn connect(
        endpoint: ReceiverEndpoint,
        trusted: bool,
        peer_versions: &[u8],
    ) -> Result<Self, FcastError> {
        endpoint.validate()?;
        if !trusted {
            return Err(FcastError::ReceiverNotTrusted);
        }
        let version = negotiate_version(peer_versions)?;
        Ok(Self {
            endpoint,
            state: ConnectionState::Connected,
            version: Some(version),
        })
    }

    pub fn endpoint(&self) -> &ReceiverEndpoint {
        &self.endpoint
    }

    pub fn state(&self) -> ConnectionState {
        self.state
    }

    pub fn version(&self) -> Option<FcastVersion> {
        self.version
    }

    pub fn command(&self, command: FcastCommand) -> Result<WireMessage, FcastError> {
        if self.state != ConnectionState::Connected {
            return Err(FcastError::NotConnected);
        }
        let payload = serde_json::to_vec(&command).expect("typed FCast commands serialize");
        WireMessage::new(0x14, payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireMessage {
    pub opcode: u8,
    pub payload: Vec<u8>,
}

impl WireMessage {
    pub fn new(opcode: u8, payload: Vec<u8>) -> Result<Self, FcastError> {
        if payload.len() + 5 > MAX_PACKET_BYTES {
            return Err(FcastError::PacketTooLarge {
                actual: payload.len() + 5,
                maximum: MAX_PACKET_BYTES,
            });
        }
        Ok(Self { opcode, payload })
    }

    pub fn encode(&self) -> Vec<u8> {
        let length = (self.payload.len() + 1) as u32;
        let mut encoded = Vec::with_capacity(self.payload.len() + 5);
        encoded.extend_from_slice(&length.to_be_bytes());
        encoded.push(self.opcode);
        encoded.extend_from_slice(&self.payload);
        encoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn endpoint() -> ReceiverEndpoint {
        ReceiverEndpoint {
            id: "receiver-1".into(),
            name: "Living room".into(),
            host: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            port: 1234,
            fingerprint: "sha256/fingerprint".into(),
        }
    }

    #[test]
    fn legacy_downgrade_is_rejected() {
        assert_eq!(negotiate_version(&[3]), Err(FcastError::UnsupportedVersion));
        assert_eq!(negotiate_version(&[3, 4]), Ok(FcastVersion::V4));
    }

    #[test]
    fn commands_have_a_bounded_wire_representation() {
        let session = FcastSession::connect(endpoint(), true, &[4]).unwrap();
        let message = session
            .command(FcastCommand::Load {
                request: LoadRequest {
                    url: "https://media.example/video.mp4".into(),
                    title: Some("Video".into()),
                    content_type: Some("video/mp4".into()),
                },
            })
            .unwrap();
        assert_eq!(message.encode()[4], 0x14);
    }

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
