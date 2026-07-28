//! Adapter for the pinned upstream FCast sender SDK.

use crate::{FcastError, LoadRequest, ReceiverEndpoint};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use fcast_sender_sdk::device::{
    ApplicationInfo, CastingDevice, DeviceConnectionState, DeviceEventHandler, DeviceInfo,
    FWRTCSignaller, MediaTrack, MediaTrackType, MirroringOfferSink, PlaybackState, ProtocolType,
    QueueState, ReceiverError, Source, TrackList,
};
use fcast_sender_sdk::{DeviceDiscovererEventHandler, IpAddr, context::CastContext};
use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr as StdIpAddr;
use std::sync::{Arc, Mutex};

pub struct UpstreamContext {
    context: CastContext,
}

pub type DeviceHandle = Arc<dyn CastingDevice>;

#[derive(Debug, Clone)]
pub enum UpstreamEvent {
    ConnectionState(String),
    Volume(f64),
    Time(f64),
    PlaybackState(String),
    Duration(f64),
    Speed(f64),
    PlaybackStopped,
    PlaybackError(String),
    Tracks(TrackSnapshot),
    TrackSelected { id: Option<u32>, kind: String },
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
    pub kind: String,
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
        self.emit(UpstreamEvent::ConnectionState(format!("{state:?}")));
    }

    fn volume_changed(&self, volume: f64) {
        self.emit(UpstreamEvent::Volume(volume));
    }

    fn time_changed(&self, time: f64) {
        self.emit(UpstreamEvent::Time(time));
    }

    fn playback_state_changed(&self, state: PlaybackState) {
        self.emit(UpstreamEvent::PlaybackState(format!("{state:?}")));
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
            kind: track_type_name(&typ).to_owned(),
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
                kind: track_type_name(&track.typ).to_owned(),
            })
            .collect(),
        selected_video,
        selected_audio,
        selected_subtitle,
    }
}

fn track_type_name(track_type: &MediaTrackType) -> &'static str {
    match track_type {
        MediaTrackType::Video => "video",
        MediaTrackType::Audio => "audio",
        MediaTrackType::Subtitle => "text",
    }
}

impl UpstreamContext {
    pub fn new() -> Result<Self, String> {
        CastContext::new()
            .map(|context| Self { context })
            .map_err(|error| error.to_string())
    }

    pub fn device(&self, endpoint: &ReceiverEndpoint) -> Result<DeviceHandle, FcastError> {
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
            .map_err(|error| {
                FcastError::InvalidEndpoint(format!("invalid fingerprint: {error}"))
            })?;
        if fingerprint_bytes.len() != 32 {
            return Err(FcastError::InvalidEndpoint(
                "fingerprint must be a SHA-256 digest".into(),
            ));
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

    pub fn connect(&self, endpoint: &ReceiverEndpoint) -> Result<DeviceHandle, String> {
        self.connect_with_events(endpoint, Arc::new(|_| {}))
    }

    pub fn connect_with_events(
        &self,
        endpoint: &ReceiverEndpoint,
        sink: Arc<dyn Fn(UpstreamEvent) + Send + Sync>,
    ) -> Result<DeviceHandle, String> {
        let device = self.device(endpoint).map_err(|error| error.to_string())?;
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
            .map_err(|error| error.to_string())?;
        Ok(device)
    }

    pub fn load(&self, device: &DeviceHandle, request: &LoadRequest) -> Result<(), String> {
        let url = request.validate().map_err(|error| error.to_string())?;
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
            .map_err(|error| error.to_string())
    }

    pub fn load_file(
        &self,
        device: &DeviceHandle,
        request: &LoadRequest,
        path: &std::path::Path,
        content_type: &str,
    ) -> Result<(), String> {
        request.validate().map_err(|error| error.to_string())?;
        if !path.is_file() {
            return Err(format!(
                "companion resource does not exist: {}",
                path.display()
            ));
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
            .map_err(|error| error.to_string())
    }

    pub fn connect_and_load(
        &self,
        endpoint: &ReceiverEndpoint,
        request: &LoadRequest,
    ) -> Result<(), String> {
        let device = self.connect(endpoint)?;
        self.load(&device, request)
    }

    pub fn control(
        device: &DeviceHandle,
        action: &str,
        position_ms: Option<f64>,
        level: Option<f64>,
        speed: Option<f64>,
    ) -> Result<(), String> {
        let result = match action {
            "play" | "resume" => device.resume_playback(),
            "pause" => device.pause_playback(),
            "stop" => device.stop_playback(),
            "seek" => position_ms
                .map(|position| device.seek(position / 1000.0))
                .unwrap_or_else(|| {
                    Err(fcast_sender_sdk::device::CastingDeviceError::FailedToSendCommand)
                }),
            "volume" => level
                .map(|level| device.change_volume(level))
                .unwrap_or_else(|| {
                    Err(fcast_sender_sdk::device::CastingDeviceError::FailedToSendCommand)
                }),
            "speed" => speed
                .map(|speed| device.change_speed(speed))
                .unwrap_or_else(|| {
                    Err(fcast_sender_sdk::device::CastingDeviceError::FailedToSendCommand)
                }),
            _ => return Err(format!("unsupported control action '{action}'")),
        };
        result.map_err(|error| error.to_string())
    }

    pub fn select_track(
        device: &DeviceHandle,
        track_id: Option<u32>,
        track_type: &str,
    ) -> Result<(), String> {
        let typ = match track_type {
            "video" => MediaTrackType::Video,
            "audio" => MediaTrackType::Audio,
            "text" | "subtitle" => MediaTrackType::Subtitle,
            _ => return Err(format!("unsupported track type '{track_type}'")),
        };
        device
            .change_track(track_id, typ)
            .map_err(|error| error.to_string())
    }

    pub fn start_mirroring(
        device: &DeviceHandle,
        signaller: Arc<dyn FWRTCSignaller>,
    ) -> Result<(), String> {
        device
            .start_mirroring_session(signaller)
            .map_err(|error| error.to_string())
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

    pub fn submit_offer(&self, sdp: String) -> Result<(), String> {
        if sdp.trim().is_empty() || sdp.len() > 256 * 1024 {
            return Err("SDP offer is empty or too large".into());
        }
        let sink = self
            .offer_sink
            .lock()
            .map_err(|_| "mirroring signaller lock poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "receiver has not prepared mirroring yet".to_owned())?;
        sink.send_offer(sdp);
        Ok(())
    }
}

impl FWRTCSignaller for BrowserSignaller {
    fn set_offer_sink(&self, sink: Arc<MirroringOfferSink>) {
        if let Ok(mut current) = self.offer_sink.lock() {
            *current = Some(sink);
            (self.on_ready)(self.session_id);
        }
    }

    fn on_answer_received(&self, answer: String) {
        (self.on_answer)(self.session_id, answer);
    }
}

pub struct NoopDeviceEvents;

impl DeviceEventHandler for NoopDeviceEvents {
    fn connection_state_changed(&self, _state: DeviceConnectionState) {}
    fn volume_changed(&self, _volume: f64) {}
    fn time_changed(&self, _time: f64) {}
    fn playback_state_changed(&self, _state: PlaybackState) {}
    fn duration_changed(&self, _duration: f64) {}
    fn speed_changed(&self, _speed: f64) {}
    fn source_changed(&self, _source: Source) {}
    fn playback_stopped(&self) {}
    fn playback_error(&self, _message: String) {}
    fn tracks_available(&self, _tracks: Vec<MediaTrack>) {}
    fn track_selected(&self, _id: Option<u32>, _typ: MediaTrackType) {}
    fn tracks_changed(&self, _tracks: TrackList) {}
    fn queue_changed(&self, _queue: QueueState) {}
    fn command_error(&self, _error: ReceiverError) {}
}

pub struct NoopDiscoveryEvents;

impl DeviceDiscovererEventHandler for NoopDiscoveryEvents {
    fn device_available(&self, _device_info: DeviceInfo) {}
    fn device_removed(&self, _device_name: String) {}
    fn device_changed(&self, _device_info: DeviceInfo) {}
}
