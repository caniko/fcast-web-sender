//! Adapter for the pinned upstream FCast sender SDK.

use crate::{FcastError, LoadRequest, ReceiverEndpoint};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use fcast_sender_sdk::device::{
    ApplicationInfo, CastingDevice, CastingDeviceError, DeviceConnectionState, DeviceEventHandler,
    DeviceInfo, FWRTCSignaller, MediaTrack, MediaTrackType, MirroringOfferSink,
    PlaybackState as SdkPlaybackState, ProtocolType, QueueState, ReceiverError, Source, TrackList,
};
use fcast_sender_sdk::{AsyncRuntimeError, IpAddr, context::CastContext};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr as StdIpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use thiserror::Error;

pub struct UpstreamContext {
    context: CastContext,
}

pub type DeviceHandle = Arc<dyn CastingDevice>;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum UpstreamError {
    #[error("failed to initialize the FCast runtime: {0}")]
    Runtime(#[source] AsyncRuntimeError),
    #[error(transparent)]
    Client(#[from] FcastError),
    #[error("invalid receiver fingerprint: {0}")]
    Fingerprint(#[source] base64::DecodeError),
    #[error("FCast SDK operation failed: {0}")]
    Device(#[source] CastingDeviceError),
    #[error("companion resource does not exist: {0}")]
    MissingResource(PathBuf),
    #[error("unsupported control action '{0}'")]
    UnsupportedControlAction(String),
    #[error("control action '{0}' requires a value")]
    MissingControlValue(&'static str),
    #[error("SDP offer is empty or too large")]
    InvalidOffer,
    #[error("mirroring signaller lock poisoned")]
    SignallerLockPoisoned,
    #[error("receiver has not prepared mirroring yet")]
    SignallerNotReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Reconnecting,
    Connected,
}

impl From<DeviceConnectionState> for ConnectionState {
    fn from(state: DeviceConnectionState) -> Self {
        match state {
            DeviceConnectionState::Disconnected => Self::Disconnected,
            DeviceConnectionState::Connecting => Self::Connecting,
            DeviceConnectionState::Reconnecting => Self::Reconnecting,
            DeviceConnectionState::Connected { .. } => Self::Connected,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackState {
    Idle,
    Buffering,
    Playing,
    Paused,
    Ended,
}

impl From<SdkPlaybackState> for PlaybackState {
    fn from(state: SdkPlaybackState) -> Self {
        match state {
            SdkPlaybackState::Idle => Self::Idle,
            SdkPlaybackState::Buffering => Self::Buffering,
            SdkPlaybackState::Playing => Self::Playing,
            SdkPlaybackState::Paused => Self::Paused,
            SdkPlaybackState::Ended => Self::Ended,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackType {
    Video,
    Audio,
    Text,
}

impl From<MediaTrackType> for TrackType {
    fn from(track_type: MediaTrackType) -> Self {
        match track_type {
            MediaTrackType::Video => Self::Video,
            MediaTrackType::Audio => Self::Audio,
            MediaTrackType::Subtitle => Self::Text,
        }
    }
}

impl From<TrackType> for MediaTrackType {
    fn from(track_type: TrackType) -> Self {
        match track_type {
            TrackType::Video => Self::Video,
            TrackType::Audio => Self::Audio,
            TrackType::Text => Self::Subtitle,
        }
    }
}

#[derive(Debug, Clone)]
pub enum UpstreamEvent {
    ConnectionState(ConnectionState),
    Volume(f64),
    Time(f64),
    PlaybackState(PlaybackState),
    Duration(f64),
    Speed(f64),
    PlaybackStopped,
    PlaybackError(String),
    Tracks(TrackSnapshot),
    TrackSelected { id: Option<u32>, kind: TrackType },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackSnapshot {
    pub tracks: Vec<TrackInfo>,
    pub selected_video: Option<u32>,
    pub selected_audio: Option<u32>,
    pub selected_subtitle: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub id: u32,
    pub title: Option<String>,
    pub language: String,
    pub kind: TrackType,
}

struct ForwardingDeviceEvents {
    sink: Arc<dyn Fn(UpstreamEvent) + Send + Sync>,
}

impl ForwardingDeviceEvents {
    fn emit(&self, event: UpstreamEvent) {
        (self.sink)(event);
    }
}

impl DeviceEventHandler for ForwardingDeviceEvents {
    fn connection_state_changed(&self, state: DeviceConnectionState) {
        self.emit(UpstreamEvent::ConnectionState(state.into()));
    }

    fn volume_changed(&self, volume: f64) {
        self.emit(UpstreamEvent::Volume(volume));
    }

    fn time_changed(&self, time: f64) {
        self.emit(UpstreamEvent::Time(time));
    }

    fn playback_state_changed(&self, state: SdkPlaybackState) {
        self.emit(UpstreamEvent::PlaybackState(state.into()));
    }

    fn duration_changed(&self, duration: f64) {
        self.emit(UpstreamEvent::Duration(duration));
    }

    fn speed_changed(&self, speed: f64) {
        self.emit(UpstreamEvent::Speed(speed));
    }

    fn source_changed(&self, _source: Source) {}

    fn playback_stopped(&self) {
        self.emit(UpstreamEvent::PlaybackStopped);
    }

    fn playback_error(&self, message: String) {
        self.emit(UpstreamEvent::PlaybackError(message));
    }

    fn tracks_available(&self, tracks: Vec<MediaTrack>) {
        self.emit(UpstreamEvent::Tracks(track_snapshot(
            tracks, None, None, None,
        )));
    }

    fn track_selected(&self, id: Option<u32>, typ: MediaTrackType) {
        self.emit(UpstreamEvent::TrackSelected {
            id,
            kind: typ.into(),
        });
    }

    fn tracks_changed(&self, tracks: TrackList) {
        self.emit(UpstreamEvent::Tracks(track_snapshot(
            tracks.tracks,
            tracks.selected_video,
            tracks.selected_audio,
            tracks.selected_subtitle,
        )));
    }

    fn queue_changed(&self, _queue: QueueState) {}

    fn command_error(&self, error: ReceiverError) {
        self.emit(UpstreamEvent::PlaybackError(format!(
            "receiver rejected command: {error:?}"
        )));
    }
}

fn track_snapshot(
    tracks: Vec<MediaTrack>,
    selected_video: Option<u32>,
    selected_audio: Option<u32>,
    selected_subtitle: Option<u32>,
) -> TrackSnapshot {
    TrackSnapshot {
        tracks: tracks
            .into_iter()
            .map(|track| TrackInfo {
                id: track.id,
                title: track.title,
                language: track.language,
                kind: track.typ.into(),
            })
            .collect(),
        selected_video,
        selected_audio,
        selected_subtitle,
    }
}

impl UpstreamContext {
    pub fn new() -> Result<Self, UpstreamError> {
        CastContext::new()
            .map(|context| Self { context })
            .map_err(UpstreamError::Runtime)
    }

    fn device(&self, endpoint: &ReceiverEndpoint) -> Result<DeviceHandle, UpstreamError> {
        endpoint.validate()?;
        let address = match endpoint.host {
            StdIpAddr::V4(address) => IpAddr::v4(
                address.octets()[0],
                address.octets()[1],
                address.octets()[2],
                address.octets()[3],
            ),
            StdIpAddr::V6(address) => {
                let segments = address.segments();
                IpAddr::V6 {
                    o1: (segments[0] >> 8) as u8,
                    o2: segments[0] as u8,
                    o3: (segments[1] >> 8) as u8,
                    o4: segments[1] as u8,
                    o5: (segments[2] >> 8) as u8,
                    o6: segments[2] as u8,
                    o7: (segments[3] >> 8) as u8,
                    o8: segments[3] as u8,
                    o9: (segments[4] >> 8) as u8,
                    o10: segments[4] as u8,
                    o11: (segments[5] >> 8) as u8,
                    o12: segments[5] as u8,
                    o13: (segments[6] >> 8) as u8,
                    o14: segments[6] as u8,
                    o15: (segments[7] >> 8) as u8,
                    o16: segments[7] as u8,
                    scope_id: 0,
                }
            }
        };
        let encoded_fingerprint = endpoint
            .fingerprint
            .strip_prefix("sha256/")
            .unwrap_or(&endpoint.fingerprint);
        let fingerprint_bytes = STANDARD_NO_PAD
            .decode(encoded_fingerprint)
            .or_else(|_| STANDARD.decode(encoded_fingerprint))
            .map_err(UpstreamError::Fingerprint)?;
        if fingerprint_bytes.len() != 32 {
            return Err(
                FcastError::InvalidEndpoint("fingerprint must be a SHA-256 digest".into()).into(),
            );
        }
        // The SDK reads the TXT value with padded standard base64. The trust
        // store intentionally keeps its public representation unpadded, so
        // normalize at this adapter boundary.
        let fingerprint = STANDARD.encode(fingerprint_bytes);
        Ok(self.context.create_device_from_info(DeviceInfo {
            name: endpoint.name.clone(),
            protocol: ProtocolType::FCast,
            addresses: vec![address],
            port: endpoint.port,
            txt_records: HashMap::from([("fp".into(), fingerprint)]),
        }))
    }

    pub fn connect_with_events(
        &self,
        endpoint: &ReceiverEndpoint,
        sink: Arc<dyn Fn(UpstreamEvent) + Send + Sync>,
    ) -> Result<DeviceHandle, UpstreamError> {
        let device = self.device(endpoint)?;
        device
            .connect(
                Some(ApplicationInfo {
                    name: "fcast-web-sender".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    display_name: "FCast Web Sender".into(),
                }),
                Arc::new(ForwardingDeviceEvents { sink }),
                1000,
            )
            .map_err(UpstreamError::Device)?;
        Ok(device)
    }

    pub fn load(&self, device: &DeviceHandle, request: &LoadRequest) -> Result<(), UpstreamError> {
        let url = request.validate()?;
        device
            .load(
                fcast_sender_sdk::device::LoadRequest::Video {
                    content_type: request
                        .content_type
                        .clone()
                        .unwrap_or_else(|| "video/mp4".into()),
                    url: url.to_string(),
                    resume_position: 0.0,
                    speed: None,
                    volume: None,
                    metadata: request.title.clone().map(|title| {
                        fcast_sender_sdk::device::Metadata {
                            title: Some(title),
                            thumbnail_url: None,
                        }
                    }),
                    request_headers: None,
                },
                Some(500),
            )
            .map_err(UpstreamError::Device)
    }

    pub fn load_file(
        &self,
        device: &DeviceHandle,
        request: &LoadRequest,
        path: &std::path::Path,
        content_type: &str,
    ) -> Result<(), UpstreamError> {
        request.validate()?;
        if !path.is_file() {
            return Err(UpstreamError::MissingResource(path.to_owned()));
        }
        device
            .load(
                fcast_sender_sdk::device::LoadRequest::CompanionResource {
                    content_type: content_type.to_owned(),
                    source: fcast_sender_sdk::device::CompanionSource::from_path(
                        path.to_string_lossy().to_string(),
                        content_type,
                    ),
                    resume_position: Some(0.0),
                    speed: None,
                    volume: None,
                    metadata: request.title.clone().map(|title| {
                        fcast_sender_sdk::device::Metadata {
                            title: Some(title),
                            thumbnail_url: None,
                        }
                    }),
                },
                Some(500),
            )
            .map_err(UpstreamError::Device)
    }

    pub fn control(
        device: &DeviceHandle,
        action: &str,
        position_ms: Option<f64>,
        level: Option<f64>,
        speed: Option<f64>,
    ) -> Result<(), UpstreamError> {
        match action {
            "play" | "resume" => device.resume_playback().map_err(UpstreamError::Device),
            "pause" => device.pause_playback().map_err(UpstreamError::Device),
            "stop" => device.stop_playback().map_err(UpstreamError::Device),
            "seek" => position_ms
                .ok_or(UpstreamError::MissingControlValue("seek"))
                .and_then(|position| {
                    device
                        .seek(position / 1000.0)
                        .map_err(UpstreamError::Device)
                }),
            "volume" => level
                .ok_or(UpstreamError::MissingControlValue("volume"))
                .and_then(|level| device.change_volume(level).map_err(UpstreamError::Device)),
            "speed" => speed
                .ok_or(UpstreamError::MissingControlValue("speed"))
                .and_then(|speed| device.change_speed(speed).map_err(UpstreamError::Device)),
            _ => Err(UpstreamError::UnsupportedControlAction(action.to_owned())),
        }
    }

    pub fn select_track(
        device: &DeviceHandle,
        track_id: Option<u32>,
        track_type: TrackType,
    ) -> Result<(), UpstreamError> {
        device
            .change_track(track_id, track_type.into())
            .map_err(UpstreamError::Device)
    }

    pub fn start_mirroring(
        device: &DeviceHandle,
        signaller: Arc<dyn FWRTCSignaller>,
    ) -> Result<(), UpstreamError> {
        device
            .start_mirroring_session(signaller)
            .map_err(UpstreamError::Device)
    }
}

pub struct BrowserSignaller {
    session_id: u16,
    offer_sink: Mutex<Option<Arc<MirroringOfferSink>>>,
    on_answer: Arc<dyn Fn(u16, String) + Send + Sync>,
    on_ready: Arc<dyn Fn(u16) + Send + Sync>,
}

impl std::fmt::Debug for BrowserSignaller {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserSignaller")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

impl BrowserSignaller {
    pub fn new(
        session_id: u16,
        on_answer: Arc<dyn Fn(u16, String) + Send + Sync>,
        on_ready: Arc<dyn Fn(u16) + Send + Sync>,
    ) -> Self {
        Self {
            session_id,
            offer_sink: Mutex::new(None),
            on_answer,
            on_ready,
        }
    }

    pub fn submit_offer(&self, sdp: String) -> Result<(), UpstreamError> {
        if sdp.trim().is_empty() || sdp.len() > 256 * 1024 {
            return Err(UpstreamError::InvalidOffer);
        }
        let sink = self
            .offer_sink
            .lock()
            .map_err(|_| UpstreamError::SignallerLockPoisoned)?
            .clone()
            .ok_or(UpstreamError::SignallerNotReady)?;
        sink.send_offer(sdp);
        Ok(())
    }

    fn update_offer_sink(&self, sink: Option<Arc<MirroringOfferSink>>) {
        if let Ok(mut current) = self.offer_sink.lock() {
            *current = sink;
        } else {
            return;
        }
        (self.on_ready)(self.session_id);
    }
}

impl FWRTCSignaller for BrowserSignaller {
    fn set_offer_sink(&self, sink: Arc<MirroringOfferSink>) {
        self.update_offer_sink(Some(sink));
    }

    fn on_answer_received(&self, answer: String) {
        (self.on_answer)(self.session_id, answer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Weak;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn sdk_states_map_to_stable_serialized_values() {
        let connected = DeviceConnectionState::Connected {
            used_remote_addr: IpAddr::v4(192, 0, 2, 1),
            local_addr: IpAddr::v4(192, 0, 2, 2),
            capabilities: None,
        };
        let connection_states = [
            (
                DeviceConnectionState::Disconnected,
                ConnectionState::Disconnected,
                "\"disconnected\"",
            ),
            (
                DeviceConnectionState::Connecting,
                ConnectionState::Connecting,
                "\"connecting\"",
            ),
            (
                DeviceConnectionState::Reconnecting,
                ConnectionState::Reconnecting,
                "\"reconnecting\"",
            ),
            (connected, ConnectionState::Connected, "\"connected\""),
        ];
        for (sdk, local, json) in connection_states {
            assert_eq!(ConnectionState::from(sdk), local);
            assert_eq!(serde_json::to_string(&local).unwrap(), json);
        }

        let playback_states = [
            (SdkPlaybackState::Idle, PlaybackState::Idle, "\"idle\""),
            (
                SdkPlaybackState::Buffering,
                PlaybackState::Buffering,
                "\"buffering\"",
            ),
            (
                SdkPlaybackState::Playing,
                PlaybackState::Playing,
                "\"playing\"",
            ),
            (
                SdkPlaybackState::Paused,
                PlaybackState::Paused,
                "\"paused\"",
            ),
            (SdkPlaybackState::Ended, PlaybackState::Ended, "\"ended\""),
        ];
        for (sdk, local, json) in playback_states {
            assert_eq!(PlaybackState::from(sdk), local);
            assert_eq!(serde_json::to_string(&local).unwrap(), json);
        }

        let track_types = [
            (MediaTrackType::Video, TrackType::Video, "\"video\""),
            (MediaTrackType::Audio, TrackType::Audio, "\"audio\""),
            (MediaTrackType::Subtitle, TrackType::Text, "\"text\""),
        ];
        for (sdk, local, json) in track_types {
            assert_eq!(TrackType::from(sdk), local);
            assert_eq!(serde_json::to_string(&local).unwrap(), json);
        }
    }

    #[test]
    fn ready_callback_can_reenter_offer_sink_mutex() {
        let reentered = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&reentered);
        let signaller = Arc::new_cyclic(|weak: &Weak<BrowserSignaller>| {
            let weak = weak.clone();
            BrowserSignaller::new(
                1,
                Arc::new(|_, _| {}),
                Arc::new(move |_| {
                    observed.store(
                        weak.upgrade().unwrap().offer_sink.try_lock().is_ok(),
                        Ordering::Relaxed,
                    );
                }),
            )
        });

        signaller.update_offer_sink(None);

        assert!(reentered.load(Ordering::Relaxed));
    }
}
