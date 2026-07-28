//! The bounded, control-only protocol used between an extension and the local
//! companion. Media bytes never use this channel.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use thiserror::Error;

mod generated;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 1_048_576;
pub const MAX_ID_BYTES: usize = 128;
pub const MAX_STRING_BYTES: usize = 16 * 1024;
pub const MAX_COLLECTION_ITEMS: usize = 128;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("native-messaging frame header was truncated")]
    TruncatedHeader,
    #[error("native-messaging frame body was truncated")]
    TruncatedBody,
    #[error("native-messaging frame is {actual} bytes; maximum is {maximum}")]
    TooLarge { actual: usize, maximum: usize },
    #[error("I/O while reading native-messaging frame: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("invalid JSON request: {0}")]
    Json(#[from] serde_json::Error),
    #[error("request id must be a non-empty string of at most {MAX_ID_BYTES} bytes")]
    InvalidRequestId,
    #[error("unsupported bridge protocol version {0}; expected {PROTOCOL_VERSION}")]
    UnsupportedVersion(u32),
    #[error("method must be a non-empty string")]
    InvalidMethod,
    #[error("unknown bridge method '{0}'")]
    UnknownMethod(String),
    #[error("params must be a JSON object")]
    InvalidParams,
    #[error("request contains a value beyond the bridge limits: {0}")]
    ValueLimit(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub v: u32,
    pub id: String,
    pub method: String,
    pub params: Map<String, Value>,
}

impl Request {
    pub fn parse(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let request: Self = serde_json::from_slice(bytes)?;
        if request.v != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(request.v));
        }
        if request.id.is_empty() || request.id.len() > MAX_ID_BYTES {
            return Err(ProtocolError::InvalidRequestId);
        }
        if request.method.is_empty() {
            return Err(ProtocolError::InvalidMethod);
        }
        if !generated::GENERATED_BRIDGE_METHODS.contains(&request.method.as_str()) {
            return Err(ProtocolError::UnknownMethod(request.method));
        }
        validate_json(&Value::Object(request.params.clone()), 0)?;
        Ok(request)
    }

    pub fn param_string(&self, name: &str) -> Result<&str, BridgeError> {
        self.params
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                BridgeError::InvalidParams(format!("'{name}' must be a non-empty string"))
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub v: u32,
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResponseError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub recoverable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_next_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Map<String, Value>>,
}

impl Response {
    pub fn success(id: impl Into<String>, result: Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: impl Into<String>, error: BridgeError) -> Self {
        let code = error.code().to_owned();
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
            ok: false,
            result: None,
            error: Some(ResponseError {
                code,
                message: error.to_string(),
                recoverable: false,
                suggested_next_mode: None,
                details: None,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub v: u32,
    pub event: String,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiverSummary {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub fingerprint: Option<String>,
    pub connection_state: ConnectionState,
    pub trusted: bool,
    pub ttl_seconds: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    Discovered,
    Connecting,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub receiver_id: String,
    pub state: SessionState,
    pub media_url: Option<String>,
    pub revision: u64,
    pub position_ms: f64,
    pub volume: f64,
    pub speed: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    Loading,
    Playing,
    Paused,
    Stopped,
    Failed,
}

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("unknown bridge method '{0}'")]
    UnknownMethod(String),
    #[error("invalid parameters: {0}")]
    InvalidParams(String),
    #[error("receiver '{0}' was not found")]
    ReceiverNotFound(String),
    #[error("session '{0}' was not found")]
    SessionNotFound(String),
    #[error("receiver '{0}' is not trusted")]
    ReceiverNotTrusted(String),
    #[error("operation '{0}' is not available in this bridge build")]
    Unsupported(String),
}

fn validate_json(value: &Value, depth: usize) -> Result<(), ProtocolError> {
    if depth > 16 {
        return Err(ProtocolError::ValueLimit(
            "maximum nesting depth is 16".into(),
        ));
    }
    match value {
        Value::String(value) if value.len() > MAX_STRING_BYTES => Err(ProtocolError::ValueLimit(
            format!("string exceeds {MAX_STRING_BYTES} bytes"),
        )),
        Value::Array(values) if values.len() > MAX_COLLECTION_ITEMS => Err(
            ProtocolError::ValueLimit(format!("array exceeds {MAX_COLLECTION_ITEMS} items")),
        ),
        Value::Object(values) if values.len() > MAX_COLLECTION_ITEMS => Err(
            ProtocolError::ValueLimit(format!("object exceeds {MAX_COLLECTION_ITEMS} properties")),
        ),
        Value::Array(values) => values
            .iter()
            .try_for_each(|value| validate_json(value, depth + 1)),
        Value::Object(values) => values.iter().try_for_each(|(key, value)| {
            if key.len() > MAX_STRING_BYTES {
                return Err(ProtocolError::ValueLimit(format!(
                    "object key exceeds {MAX_STRING_BYTES} bytes"
                )));
            }
            validate_json(value, depth + 1)
        }),
        _ => Ok(()),
    }
}

impl BridgeError {
    fn code(&self) -> &'static str {
        match self {
            Self::UnknownMethod(_) => "unknown_method",
            Self::InvalidParams(_) => "invalid_params",
            Self::ReceiverNotFound(_) => "receiver_not_found",
            Self::SessionNotFound(_) => "session_not_found",
            Self::ReceiverNotTrusted(_) => "receiver_not_trusted",
            Self::Unsupported(_) => "unsupported",
        }
    }
}

#[derive(Debug)]
pub struct BridgeState {
    started_at: std::time::Instant,
    pub discovery_running: bool,
    pub receivers: BTreeMap<String, ReceiverSummary>,
    pub sessions: BTreeMap<String, SessionSummary>,
    pub trusted_fingerprints: BTreeSet<String>,
    pub companion_port: Option<u16>,
}

impl BridgeState {
    pub fn new() -> Self {
        Self {
            started_at: std::time::Instant::now(),
            discovery_running: false,
            receivers: BTreeMap::new(),
            sessions: BTreeMap::new(),
            trusted_fingerprints: BTreeSet::new(),
            companion_port: None,
        }
    }

    pub fn uptime_ms(&self) -> u128 {
        self.started_at.elapsed().as_millis()
    }

    pub fn status_json(&self) -> Value {
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "discoveryRunning": self.discovery_running,
            "receivers": self.receivers.len(),
            "sessions": self.sessions.len(),
            "companionPort": self.companion_port,
        })
    }
}

impl Default for BridgeState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn dispatch(state: &mut BridgeState, request: &Request) -> Response {
    match dispatch_inner(state, request) {
        Ok(result) => Response::success(&request.id, result),
        Err(error) => Response::failure(&request.id, error),
    }
}

fn dispatch_inner(state: &mut BridgeState, request: &Request) -> Result<Value, BridgeError> {
    match request.method.as_str() {
        "bridge.hello" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "bridgeVersion": env!("CARGO_PKG_VERSION"),
            "capabilities": [
                "discovery", "receiverTrust", "directFcast", "fcompanion",
                "diagnostics", "controlOnlyNativeMessaging"
            ],
            "limits": {"maxFrameBytes": MAX_FRAME_BYTES}
        })),
        "bridge.status" => Ok(state.status_json()),
        "discovery.start" => {
            state.discovery_running = true;
            Ok(json!({"running": true}))
        }
        "discovery.stop" => {
            state.discovery_running = false;
            Ok(json!({"running": false}))
        }
        "receiver.list" => Ok(
            serde_json::to_value(state.receivers.values().collect::<Vec<_>>())
                .expect("receiver summaries are serializable"),
        ),
        "receiver.connect" => {
            let receiver_id = request.param_string("receiverId")?.to_owned();
            let receiver = state
                .receivers
                .get_mut(&receiver_id)
                .ok_or_else(|| BridgeError::ReceiverNotFound(receiver_id.clone()))?;
            if !receiver.trusted {
                return Err(BridgeError::ReceiverNotTrusted(receiver_id));
            }
            receiver.connection_state = ConnectionState::Connected;
            Ok(json!({"receiverId": receiver.id, "state": "connected"}))
        }
        "receiver.disconnect" => {
            let receiver_id = request.param_string("receiverId")?.to_owned();
            let receiver = state
                .receivers
                .get_mut(&receiver_id)
                .ok_or_else(|| BridgeError::ReceiverNotFound(receiver_id.clone()))?;
            receiver.connection_state = ConnectionState::Disconnected;
            Ok(json!({"receiverId": receiver.id, "state": "disconnected"}))
        }
        "receiver.trust" => {
            let receiver_id = request.param_string("receiverId")?.to_owned();
            let fingerprint = request.param_string("fingerprint")?.to_owned();
            let receiver = state
                .receivers
                .get_mut(&receiver_id)
                .ok_or_else(|| BridgeError::ReceiverNotFound(receiver_id.clone()))?;
            receiver.fingerprint = Some(fingerprint.clone());
            receiver.trusted = true;
            state.trusted_fingerprints.insert(fingerprint);
            Ok(json!({"receiverId": receiver.id, "trusted": true}))
        }
        "receiver.forget" => {
            let receiver_id = request.param_string("receiverId")?.to_owned();
            let receiver = state
                .receivers
                .get_mut(&receiver_id)
                .ok_or_else(|| BridgeError::ReceiverNotFound(receiver_id.clone()))?;
            if let Some(fingerprint) = receiver.fingerprint.take() {
                state.trusted_fingerprints.remove(&fingerprint);
            }
            receiver.trusted = false;
            Ok(json!({"receiverId": receiver.id, "trusted": false}))
        }
        "session.load" => {
            let receiver_id = request.param_string("receiverId")?.to_owned();
            let media_url = request.param_string("url")?.to_owned();
            let receiver = state
                .receivers
                .get(&receiver_id)
                .ok_or_else(|| BridgeError::ReceiverNotFound(receiver_id.clone()))?;
            if !receiver.trusted || receiver.connection_state != ConnectionState::Connected {
                return Err(BridgeError::ReceiverNotTrusted(receiver_id));
            }
            let session_id = format!("session-{}", state.sessions.len() + 1);
            state.sessions.insert(
                session_id.clone(),
                SessionSummary {
                    id: session_id.clone(),
                    receiver_id,
                    state: SessionState::Loading,
                    media_url: Some(media_url),
                    revision: 1,
                    position_ms: 0.0,
                    volume: 1.0,
                    speed: 1.0,
                },
            );
            Ok(json!({"sessionId": session_id, "state": "loading"}))
        }
        "session.control" => {
            let session_id = request.param_string("sessionId")?.to_owned();
            let action = request.param_string("action")?;
            let session = state
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| BridgeError::SessionNotFound(session_id.clone()))?;
            match action {
                "play" | "resume" => session.state = SessionState::Playing,
                "pause" => session.state = SessionState::Paused,
                "stop" => session.state = SessionState::Stopped,
                "seek" => {
                    let position = request
                        .params
                        .get("positionMs")
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .ok_or_else(|| {
                            BridgeError::InvalidParams(
                                "'positionMs' must be a non-negative finite number".into(),
                            )
                        })?;
                    session.position_ms = position;
                }
                "volume" => {
                    let volume = request
                        .params
                        .get("level")
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
                        .ok_or_else(|| {
                            BridgeError::InvalidParams(
                                "'level' must be a finite number between 0 and 1".into(),
                            )
                        })?;
                    session.volume = volume;
                }
                "speed" => {
                    let speed = request
                        .params
                        .get("speed")
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite() && (0.25..=4.0).contains(value))
                        .ok_or_else(|| {
                            BridgeError::InvalidParams(
                                "'speed' must be a finite number between 0.25 and 4".into(),
                            )
                        })?;
                    session.speed = speed;
                }
                _ => return Err(BridgeError::InvalidParams("unknown session action".into())),
            }
            session.revision = session.revision.saturating_add(1);
            Ok(json!({
                "sessionId": session_id,
                "state": session.state,
                "revision": session.revision,
                "positionMs": session.position_ms,
                "volume": session.volume,
                "speed": session.speed,
            }))
        }
        "session.selectTrack" => {
            let session_id = request.param_string("sessionId")?;
            let track_id = request.param_string("trackId")?;
            if !state.sessions.contains_key(session_id) {
                return Err(BridgeError::SessionNotFound(session_id.to_owned()));
            }
            Ok(json!({"sessionId": session_id, "trackId": track_id}))
        }
        "session.close" => {
            let session_id = request.param_string("sessionId")?.to_owned();
            state
                .sessions
                .remove(&session_id)
                .ok_or_else(|| BridgeError::SessionNotFound(session_id.clone()))?;
            Ok(json!({"sessionId": session_id, "closed": true}))
        }
        "companion.register" => {
            let port = request
                .params
                .get("port")
                .and_then(Value::as_u64)
                .filter(|port| *port <= u16::MAX as u64)
                .ok_or_else(|| {
                    BridgeError::InvalidParams("'port' must be a valid TCP port".into())
                })?;
            state.companion_port = Some(port as u16);
            Ok(json!({"registered": true, "port": port}))
        }
        "companion.unregister" => {
            state.companion_port = None;
            Ok(json!({"registered": false}))
        }
        "diagnostics.snapshot" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "uptimeMs": state.uptime_ms(),
            "receivers": state.receivers.len(),
            "sessions": state.sessions.len(),
            "warnings": [],
            "lastError": null
        })),
        "mirror.negotiate" => Err(BridgeError::Unsupported("mirror.negotiate".into())),
        _ => Err(BridgeError::UnknownMethod(request.method.clone())),
    }
}

pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, FrameError> {
    let mut header = [0_u8; 4];
    match reader.read(&mut header[..1])? {
        0 => return Ok(None),
        1 => {}
        _ => unreachable!("a one-byte read cannot exceed one byte"),
    }
    reader
        .read_exact(&mut header[1..])
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => FrameError::TruncatedHeader,
            _ => FrameError::Io(error),
        })?;
    let length = u32::from_le_bytes(header) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge {
            actual: length,
            maximum: MAX_FRAME_BYTES,
        });
    }
    let mut body = vec![0_u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => FrameError::TruncatedBody,
            _ => FrameError::Io(error),
        })?;
    Ok(Some(body))
}

pub fn write_frame<W: Write>(writer: &mut W, body: &[u8]) -> Result<(), FrameError> {
    if body.len() > MAX_FRAME_BYTES || body.len() > u32::MAX as usize {
        return Err(FrameError::TooLarge {
            actual: body.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }
    writer.write_all(&(body.len() as u32).to_le_bytes())?;
    writer.write_all(body)?;
    writer.flush().map_err(FrameError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn request(method: &str, params: &[(&str, Value)]) -> Request {
        Request {
            v: PROTOCOL_VERSION,
            id: "1".into(),
            method: method.into(),
            params: params
                .iter()
                .map(|(key, value)| ((*key).into(), value.clone()))
                .collect(),
        }
    }

    #[test]
    fn native_message_frames_round_trip() {
        let payload = br#"{"id":"1","method":"bridge.hello","params":{}}"#;
        let mut encoded = Vec::new();
        write_frame(&mut encoded, payload).unwrap();
        assert_eq!(
            read_frame(&mut Cursor::new(encoded)).unwrap(),
            Some(payload.to_vec())
        );
    }

    #[test]
    fn oversized_frames_fail_closed() {
        let bytes = (MAX_FRAME_BYTES as u32 + 1).to_le_bytes();
        assert!(matches!(
            read_frame(&mut Cursor::new(bytes)),
            Err(FrameError::TooLarge { .. })
        ));
    }

    #[test]
    fn hello_and_trust_gate_are_stateful() {
        let mut state = BridgeState::new();
        let hello = dispatch(&mut state, &request("bridge.hello", &[]));
        assert!(hello.ok);

        state.receivers.insert(
            "r1".into(),
            ReceiverSummary {
                id: "r1".into(),
                name: "Living room".into(),
                host: "192.0.2.10".into(),
                port: 5353,
                fingerprint: None,
                connection_state: ConnectionState::Discovered,
                trusted: false,
                ttl_seconds: None,
            },
        );
        let connect = dispatch(
            &mut state,
            &request("receiver.connect", &[("receiverId", json!("r1"))]),
        );
        assert_eq!(connect.error.unwrap().code, "receiver_not_trusted");
        let trust = dispatch(
            &mut state,
            &request(
                "receiver.trust",
                &[
                    ("receiverId", json!("r1")),
                    ("fingerprint", json!("sha256/aa")),
                ],
            ),
        );
        assert!(trust.ok);
        assert!(
            dispatch(
                &mut state,
                &request("receiver.connect", &[("receiverId", json!("r1"))]),
            )
            .ok
        );
    }

    #[test]
    fn request_parser_enforces_version_method_and_value_limits() {
        let valid = serde_json::json!({
            "v": PROTOCOL_VERSION,
            "id": "1",
            "method": "bridge.hello",
            "params": {}
        });
        assert!(Request::parse(&serde_json::to_vec(&valid).unwrap()).is_ok());

        let mut unsupported = valid.clone();
        unsupported["v"] = json!(2);
        assert!(matches!(
            Request::parse(&serde_json::to_vec(&unsupported).unwrap()),
            Err(ProtocolError::UnsupportedVersion(2))
        ));

        let mut unknown = valid.clone();
        unknown["method"] = json!("bridge.nope");
        assert!(matches!(
            Request::parse(&serde_json::to_vec(&unknown).unwrap()),
            Err(ProtocolError::UnknownMethod(_))
        ));

        let mut too_large = valid;
        too_large["params"] = json!({"value": "x".repeat(MAX_STRING_BYTES + 1)});
        assert!(matches!(
            Request::parse(&serde_json::to_vec(&too_large).unwrap()),
            Err(ProtocolError::ValueLimit(_))
        ));
    }
}
