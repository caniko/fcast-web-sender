use fcast_bridge::{
    BridgeState, ConnectionState, Event, ProtocolError, ReceiverSummary, Request, Response,
    ResponseError, SessionControl, dispatch, read_frame, write_frame,
};
use fcast_client::{
    LoadRequest, ReceiverEndpoint,
    upstream::{BrowserSignaller, DeviceHandle, TrackType, UpstreamContext, UpstreamEvent},
};
use fcast_discovery::{MdnsBrowser, SERVICE_NAME};
use fcast_media_fetcher::{FetchedFile, MediaFetcher, UrlPolicy};
use fcast_receiver_trust::{Fingerprint, TrustStore};
use rand::random;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{self, stdin, stdout};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

const MAX_CREDENTIAL_LEASE_SECONDS: u64 = 300;
const MAX_CREDENTIAL_HEADER_BYTES: usize = 16 * 1024;

struct CredentialLease {
    receiver_id: String,
    fingerprint: String,
    origin: String,
    headers: Vec<(String, String)>,
    expires_at: Instant,
}

#[derive(Default)]
struct CredentialLeases {
    leases: BTreeMap<String, CredentialLease>,
}

struct CompanionContext<'a> {
    state: &'a mut BridgeState,
    trust_store: &'a mut TrustStore,
    upstream: &'a UpstreamContext,
    devices: &'a mut BTreeMap<String, DeviceHandle>,
    runtime: &'a tokio::runtime::Runtime,
    media_fetcher: &'a MediaFetcher,
    files: &'a mut BTreeMap<String, Vec<FetchedFile>>,
    output: &'a Arc<Mutex<io::Stdout>>,
    mirrors: &'a mut BTreeMap<String, Arc<BrowserSignaller>>,
    next_mirror_id: &'a mut u16,
    credential_leases: &'a mut CredentialLeases,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|argument| argument == "--version") {
        println!("fcast-companion {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let trust_path = trust_store_path()?;
    let mut trust_store = TrustStore::load(trust_path)?;
    let upstream = UpstreamContext::new().map_err(io::Error::other)?;
    let runtime = tokio::runtime::Runtime::new()?;
    let media_fetcher = MediaFetcher::new(UrlPolicy::default());
    let mut devices: BTreeMap<String, DeviceHandle> = BTreeMap::new();
    let mut files: BTreeMap<String, Vec<FetchedFile>> = BTreeMap::new();
    let mut mirrors: BTreeMap<String, Arc<BrowserSignaller>> = BTreeMap::new();
    let mut next_mirror_id = 1_u16;
    let mut credential_leases = CredentialLeases::default();

    // Native Messaging requires stdout to contain framed JSON only. Runtime
    // diagnostics belong on stderr or in the explicit diagnostics method.
    let mut input = stdin().lock();
    let output = Arc::new(Mutex::new(stdout()));
    let mut state = BridgeState::new();
    loop {
        let frame = match read_frame(&mut input) {
            Ok(frame) => frame,
            Err(error) => {
                eprintln!("fcast-companion: {error}");
                return Err(error.into());
            }
        };
        let Some(body) = frame else {
            break;
        };

        let response = match Request::parse(&body) {
            Ok(request) => {
                let mut context = CompanionContext {
                    state: &mut state,
                    trust_store: &mut trust_store,
                    upstream: &upstream,
                    devices: &mut devices,
                    runtime: &runtime,
                    media_fetcher: &media_fetcher,
                    files: &mut files,
                    output: &output,
                    mirrors: &mut mirrors,
                    next_mirror_id: &mut next_mirror_id,
                    credential_leases: &mut credential_leases,
                };
                handle_request(&mut context, &request)
            }
            Err(error) => malformed_request_response(error),
        };
        let encoded = serde_json::to_vec(&response)?;
        let mut output = output
            .lock()
            .map_err(|_| io::Error::other("native output lock poisoned"))?;
        write_frame(&mut *output, &encoded)?;
    }
    Ok(())
}

fn handle_request(context: &mut CompanionContext<'_>, request: &Request) -> Response {
    if request.method == "discovery.start" {
        discover_receivers(context.state, context.trust_store);
    }

    match request.method.as_str() {
        "receiver.trust" => trust_receiver(context.state, context.trust_store, request),
        "receiver.forget" => forget_receiver(context.state, context.trust_store, request),
        "receiver.connect" => connect_receiver(
            context.state,
            context.upstream,
            context.devices,
            context.output,
            request,
        ),
        "receiver.disconnect" => disconnect_receiver(context.state, context.devices, request),
        "session.load" => load_session(context, request),
        "session.control" => control_session(context.state, context.devices, request),
        "session.selectTrack" => select_track(context.state, context.devices, request),
        "session.close" => close_session(context.state, context.files, request),
        "diagnostics.snapshot" => diagnostics_snapshot(context.state, request),
        "credentialLease.create" => {
            create_credential_lease(context.state, context.credential_leases, request)
        }
        "credentialLease.revoke" => revoke_credential_lease(context.credential_leases, request),
        "mirror.negotiate" => mirror_negotiate(
            context.state,
            context.upstream,
            context.devices,
            context.output,
            context.mirrors,
            context.next_mirror_id,
            request,
        ),
        _ => dispatch(context.state, request),
    }
}

impl CredentialLeases {
    fn insert(&mut self, lease: CredentialLease) -> String {
        self.leases
            .retain(|_, existing| existing.expires_at > Instant::now());
        loop {
            let id = hex::encode(random::<[u8; 32]>());
            if let std::collections::btree_map::Entry::Vacant(entry) = self.leases.entry(id.clone())
            {
                entry.insert(lease);
                return id;
            }
        }
    }

    fn claim(
        &mut self,
        id: &str,
        receiver_id: &str,
        fingerprint: Option<&str>,
        url: &Url,
    ) -> Result<Vec<(String, String)>, String> {
        let lease = self
            .leases
            .get(id)
            .ok_or_else(|| "credential lease was not found".to_owned())?;
        if lease.expires_at <= Instant::now() {
            self.leases.remove(id);
            return Err("credential lease expired".into());
        }
        if lease.receiver_id != receiver_id || Some(lease.fingerprint.as_str()) != fingerprint {
            return Err("credential lease is bound to another receiver".into());
        }
        if url.scheme() != "https" || lease.origin != url.origin().ascii_serialization() {
            return Err("credential lease is bound to another HTTPS origin".into());
        }
        Ok(self
            .leases
            .remove(id)
            .ok_or_else(|| "credential lease was already claimed".to_owned())?
            .headers)
    }
}

fn create_credential_lease(
    state: &BridgeState,
    leases: &mut CredentialLeases,
    request: &Request,
) -> Response {
    let receiver_id = match request.param_string("receiverId") {
        Ok(value) => value,
        Err(error) => return Response::failure(&request.id, error),
    };
    let Some(receiver) = state.receivers.get(receiver_id) else {
        return failure(&request.id, "receiver_not_found", receiver_id);
    };
    let Some(fingerprint) = receiver.fingerprint.as_deref().filter(|_| receiver.trusted) else {
        return failure(
            &request.id,
            "receiver_not_trusted",
            "credential leases require a trusted receiver",
        );
    };
    let url = match request
        .param_string("url")
        .ok()
        .and_then(|value| Url::parse(value).ok())
        .filter(|url| url.scheme() == "https" && url.host_str().is_some())
    {
        Some(url) => url,
        None => {
            return failure(
                &request.id,
                "invalid_params",
                "credential leases require an HTTPS URL",
            );
        }
    };
    let Some(raw_headers) = request.params.get("headers").and_then(Value::as_object) else {
        return failure(
            &request.id,
            "invalid_params",
            "credential lease headers must be an object",
        );
    };
    if raw_headers.is_empty() || raw_headers.len() > 2 {
        return failure(
            &request.id,
            "invalid_params",
            "credential leases require one or two credential headers",
        );
    }
    let mut headers = Vec::with_capacity(raw_headers.len());
    for (name, value) in raw_headers {
        if !matches!(
            name.to_ascii_lowercase().as_str(),
            "authorization" | "cookie"
        ) {
            return failure(
                &request.id,
                "invalid_params",
                format!("credential header '{name}' is not allowed"),
            );
        }
        let Some(value) = value.as_str().filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_CREDENTIAL_HEADER_BYTES
                && !value.contains(['\r', '\n'])
        }) else {
            return failure(
                &request.id,
                "invalid_params",
                format!("credential header '{name}' is invalid or too large"),
            );
        };
        headers.push((name.clone(), value.to_owned()));
    }
    let ttl_seconds = request
        .params
        .get("ttlSeconds")
        .and_then(Value::as_u64)
        .unwrap_or(60);
    if !(1..=MAX_CREDENTIAL_LEASE_SECONDS).contains(&ttl_seconds) {
        return failure(
            &request.id,
            "invalid_params",
            format!("credential lease TTL must be between 1 and {MAX_CREDENTIAL_LEASE_SECONDS}"),
        );
    }
    let Some(expires_at) = Instant::now().checked_add(Duration::from_secs(ttl_seconds)) else {
        return failure(
            &request.id,
            "invalid_params",
            "credential lease TTL overflowed",
        );
    };
    let id = leases.insert(CredentialLease {
        receiver_id: receiver_id.to_owned(),
        fingerprint: fingerprint.to_owned(),
        origin: url.origin().ascii_serialization(),
        headers,
        expires_at,
    });
    Response::success(
        &request.id,
        serde_json::json!({"credentialLeaseId": id, "expiresInSeconds": ttl_seconds}),
    )
}

fn revoke_credential_lease(leases: &mut CredentialLeases, request: &Request) -> Response {
    let id = match request.param_string("credentialLeaseId") {
        Ok(value) => value,
        Err(error) => return Response::failure(&request.id, error),
    };
    Response::success(
        &request.id,
        serde_json::json!({"revoked": leases.leases.remove(id).is_some()}),
    )
}

fn discover_receivers(state: &mut BridgeState, trust_store: &TrustStore) {
    let browser = match MdnsBrowser::bind(Duration::from_millis(250)) {
        Ok(browser) => browser,
        Err(error) => {
            eprintln!("fcast-companion: discovery socket unavailable: {error}");
            state.warning(format!("discovery socket unavailable: {error}"));
            return;
        }
    };
    let records = match browser.discover(SERVICE_NAME) {
        Ok(records) => records,
        Err(error) => {
            eprintln!("fcast-companion: discovery failed: {error}");
            state.warning(format!("discovery failed: {error}"));
            return;
        }
    };
    for record in records {
        let Some(host) = record.first_address() else {
            continue;
        };
        let name = record.display_name().to_owned();
        let fingerprint = record.txt.get("fp").and_then(|raw| {
            let raw = raw.strip_prefix("sha256/").unwrap_or(raw);
            Fingerprint::try_from(format!("sha256/{raw}")).ok()
        });
        let trusted = fingerprint
            .as_ref()
            .is_some_and(|fingerprint| trust_store.is_trusted(fingerprint));
        state.receivers.insert(
            record.id.clone(),
            ReceiverSummary {
                id: record.id,
                name,
                host: host.to_string(),
                port: record.port,
                fingerprint: fingerprint.map(|value| value.to_string()),
                connection_state: ConnectionState::Discovered,
                trusted,
                ttl_seconds: Some(record.ttl_seconds),
            },
        );
    }
}

fn trust_receiver(
    state: &mut BridgeState,
    trust_store: &mut TrustStore,
    request: &Request,
) -> Response {
    let receiver_id = match request.param_string("receiverId") {
        Ok(value) => value,
        Err(error) => return Response::failure(&request.id, error),
    };
    if !state.receivers.contains_key(receiver_id) {
        return dispatch(state, request);
    }
    let raw_fingerprint = match request.param_string("fingerprint") {
        Ok(value) => value,
        Err(error) => return Response::failure(&request.id, error),
    };
    let fingerprint = match Fingerprint::try_from(raw_fingerprint) {
        Ok(value) => value,
        Err(error) => return failure(&request.id, "invalid_fingerprint", error.to_string()),
    };
    let discovered = state
        .receivers
        .get(receiver_id)
        .and_then(|receiver| receiver.fingerprint.as_deref());
    if discovered != Some(fingerprint.as_str()) {
        return failure(
            &request.id,
            "fingerprint_mismatch",
            "approved fingerprint does not match the discovered receiver",
        );
    }
    let label = request
        .params
        .get("label")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let now = match now_unix() {
        Ok(now) => now,
        Err(error) => return failure(&request.id, "clock", error.to_string()),
    };
    if let Err(error) = trust_store.trust(&fingerprint, label, now) {
        return failure(&request.id, "trust_store", error.to_string());
    }
    let mut normalized = request.clone();
    normalized
        .params
        .insert("fingerprint".into(), Value::String(fingerprint.to_string()));
    dispatch(state, &normalized)
}

fn forget_receiver(
    state: &mut BridgeState,
    trust_store: &mut TrustStore,
    request: &Request,
) -> Response {
    let receiver_id = match request.param_string("receiverId") {
        Ok(value) => value.to_owned(),
        Err(error) => return Response::failure(&request.id, error),
    };
    let fingerprint = state
        .receivers
        .get(&receiver_id)
        .and_then(|receiver| receiver.fingerprint.clone());
    if !state.receivers.contains_key(&receiver_id) {
        return dispatch(state, request);
    }
    if let Some(fingerprint) = fingerprint
        && let Err(error) = trust_store.forget(&fingerprint)
    {
        return failure(&request.id, "trust_store", error.to_string());
    }
    dispatch(state, request)
}

fn connect_receiver(
    state: &mut BridgeState,
    upstream: &UpstreamContext,
    devices: &mut BTreeMap<String, DeviceHandle>,
    output: &Arc<Mutex<io::Stdout>>,
    request: &Request,
) -> Response {
    let receiver_id = match request.param_string("receiverId") {
        Ok(value) => value.to_owned(),
        Err(error) => return Response::failure(&request.id, error),
    };
    let endpoint = match endpoint_for(state, &receiver_id) {
        Ok(endpoint) => endpoint,
        Err(error) => return failure(&request.id, error.0, error.1),
    };
    let event_output = Arc::clone(output);
    let event_receiver_id = receiver_id.clone();
    let event_sink = Arc::new(move |event| {
        let (name, data) = match event {
            UpstreamEvent::ConnectionState(state) => (
                "receiver.connectionState",
                serde_json::json!({"id": event_receiver_id.clone(), "state": state}),
            ),
            UpstreamEvent::Volume(volume) => (
                "session.volume",
                serde_json::json!({"receiverId": event_receiver_id.clone(), "volume": volume}),
            ),
            UpstreamEvent::Time(position) => (
                "session.progress",
                serde_json::json!({
                    "receiverId": event_receiver_id.clone(),
                    "positionMs": position * 1000.0
                }),
            ),
            UpstreamEvent::PlaybackState(state) => (
                "session.state",
                serde_json::json!({"receiverId": event_receiver_id.clone(), "state": state}),
            ),
            UpstreamEvent::Duration(duration) => (
                "session.progress",
                serde_json::json!({
                    "receiverId": event_receiver_id.clone(),
                    "durationMs": duration * 1000.0
                }),
            ),
            UpstreamEvent::Speed(speed) => (
                "session.speed",
                serde_json::json!({"receiverId": event_receiver_id.clone(), "speed": speed}),
            ),
            UpstreamEvent::PlaybackStopped => (
                "session.state",
                serde_json::json!({"receiverId": event_receiver_id.clone(), "state": "stopped"}),
            ),
            UpstreamEvent::PlaybackError(message) => (
                "bridge.warning",
                serde_json::json!({"receiverId": event_receiver_id.clone(), "message": message}),
            ),
            UpstreamEvent::Tracks(snapshot) => (
                "session.tracks",
                serde_json::json!({
                    "receiverId": event_receiver_id.clone(),
                    "tracks": snapshot.tracks,
                    "selectedVideo": snapshot.selected_video,
                    "selectedAudio": snapshot.selected_audio,
                    "selectedSubtitle": snapshot.selected_subtitle,
                }),
            ),
            UpstreamEvent::TrackSelected { id, kind } => (
                "session.trackSelected",
                serde_json::json!({
                    "receiverId": event_receiver_id.clone(),
                    "trackId": id,
                    "trackType": kind,
                }),
            ),
        };
        emit_event(&event_output, name, data);
    });
    let device = match upstream.connect_with_events(&endpoint, event_sink) {
        Ok(device) => device,
        Err(error) => return failure(&request.id, "fcast_connect", error.to_string()),
    };
    devices.insert(receiver_id, device);
    dispatch(state, request)
}

fn disconnect_receiver(
    state: &mut BridgeState,
    devices: &mut BTreeMap<String, DeviceHandle>,
    request: &Request,
) -> Response {
    let receiver_id = match request.param_string("receiverId") {
        Ok(value) => value.to_owned(),
        Err(error) => return Response::failure(&request.id, error),
    };
    if let Some(device) = devices.get(&receiver_id) {
        if let Err(error) = device.disconnect() {
            return failure(&request.id, "fcast_disconnect", error.to_string());
        }
        devices.remove(&receiver_id);
    }
    dispatch(state, request)
}

fn load_session(context: &mut CompanionContext<'_>, request: &Request) -> Response {
    let state = &mut *context.state;
    let upstream = context.upstream;
    let devices = &*context.devices;
    let runtime = context.runtime;
    let media_fetcher = context.media_fetcher;
    let files = &mut *context.files;
    let credential_leases = &mut *context.credential_leases;
    let receiver_id = match request.param_string("receiverId") {
        Ok(value) => value.to_owned(),
        Err(error) => return Response::failure(&request.id, error),
    };
    let url = match request.param_string("url") {
        Ok(value) => value.to_owned(),
        Err(error) => return Response::failure(&request.id, error),
    };
    let Some(device) = devices.get(&receiver_id) else {
        return failure(
            &request.id,
            "fcast_not_connected",
            "receiver is not connected",
        );
    };
    let load = LoadRequest {
        url,
        title: request
            .params
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .map(ToOwned::to_owned),
        content_type: request
            .params
            .get("contentType")
            .and_then(Value::as_str)
            .filter(|content_type| !content_type.is_empty())
            .map(ToOwned::to_owned),
    };
    let source = match load.validate() {
        Ok(source) => source,
        Err(error) => return failure(&request.id, "media_url_unavailable", error.to_string()),
    };
    let relay = request
        .params
        .get("mode")
        .and_then(Value::as_str)
        .is_some_and(|mode| mode == "fcompanion");
    if !relay {
        if let Err(error) = media_fetcher.policy().validate(&source) {
            return failure(&request.id, "media_url_unavailable", error.to_string());
        }
        if let Err(error) = upstream.load(device, &load) {
            return failure(&request.id, "fcast_load", error.to_string());
        }
        return dispatch(state, request);
    }

    if request.params.contains_key("headers") || request.params.contains_key("credentialLease") {
        return failure(
            &request.id,
            "media_headers_invalid",
            "raw headers and boolean credential leases are not accepted",
        );
    }
    let headers = match request.params.get("credentialLeaseId") {
        Some(Value::String(lease_id)) => match credential_leases.claim(
            lease_id,
            &receiver_id,
            state
                .receivers
                .get(&receiver_id)
                .and_then(|receiver| receiver.fingerprint.as_deref()),
            &source,
        ) {
            Ok(headers) => headers,
            Err(error) => return failure(&request.id, "credential_lease_invalid", error),
        },
        Some(_) => {
            return failure(
                &request.id,
                "credential_lease_invalid",
                "'credentialLeaseId' must be a string",
            );
        }
        None => Vec::new(),
    };
    let allow_sensitive = !headers.is_empty();
    let file =
        match runtime.block_on(media_fetcher.fetch_to_file(&source, &headers, allow_sensitive)) {
            Ok(file) => file,
            Err(error) => return failure(&request.id, "fcomp_resource_failed", error.to_string()),
        };
    let content_type = load
        .content_type
        .as_deref()
        .unwrap_or(&file.content_type)
        .to_owned();
    if let Err(error) = upstream.load_file(device, &load, file.path(), &content_type) {
        return failure(&request.id, "fcast_load", error.to_string());
    }
    let response = dispatch(state, request);
    if response.is_ok()
        && let Some(session_id) = response
            .result()
            .and_then(|result| result.get("sessionId"))
            .and_then(Value::as_str)
    {
        files.entry(session_id.to_owned()).or_default().push(file);
    }
    response
}

fn close_session(
    state: &mut BridgeState,
    files: &mut BTreeMap<String, Vec<FetchedFile>>,
    request: &Request,
) -> Response {
    let session_id = match request.param_string("sessionId") {
        Ok(value) => value.to_owned(),
        Err(error) => return Response::failure(&request.id, error),
    };
    let response = dispatch(state, request);
    if response.is_ok() {
        files.remove(&session_id);
    }
    response
}

fn control_session(
    state: &mut BridgeState,
    devices: &BTreeMap<String, DeviceHandle>,
    request: &Request,
) -> Response {
    let session_id = match request.param_string("sessionId") {
        Ok(value) => value.to_owned(),
        Err(error) => return Response::failure(&request.id, error),
    };
    let control = match request.session_control() {
        Ok(value) => value,
        Err(error) => return Response::failure(&request.id, error),
    };
    let Some(session) = state.sessions.get(&session_id) else {
        return dispatch(state, request);
    };
    let Some(device) = devices.get(&session.receiver_id) else {
        return failure(
            &request.id,
            "fcast_not_connected",
            "receiver is not connected",
        );
    };
    let (action, position, level, speed) = match control {
        SessionControl::Play => ("play", None, None, None),
        SessionControl::Resume => ("resume", None, None, None),
        SessionControl::Pause => ("pause", None, None, None),
        SessionControl::Stop => ("stop", None, None, None),
        SessionControl::Seek(value) => ("seek", Some(value), None, None),
        SessionControl::Volume(value) => ("volume", None, Some(value), None),
        SessionControl::Speed(value) => ("speed", None, None, Some(value)),
    };
    let result = UpstreamContext::control(device, action, position, level, speed);
    if let Err(error) = result {
        return failure(&request.id, "fcast_control", error.to_string());
    }
    dispatch(state, request)
}

fn select_track(
    state: &mut BridgeState,
    devices: &BTreeMap<String, DeviceHandle>,
    request: &Request,
) -> Response {
    let session_id = match request.param_string("sessionId") {
        Ok(value) => value.to_owned(),
        Err(error) => return Response::failure(&request.id, error),
    };
    let track_id = match request.param_string("trackId") {
        Ok("none") | Ok("off") => None,
        Ok(value) => match value.parse::<u32>() {
            Ok(value) => Some(value),
            Err(error) => return failure(&request.id, "invalid_params", error.to_string()),
        },
        Err(error) => return Response::failure(&request.id, error),
    };
    let track_type = match request.param_string("trackType") {
        Ok("video") => TrackType::Video,
        Ok("audio") => TrackType::Audio,
        Ok("text" | "subtitle") => TrackType::Text,
        Ok(_) => return failure(&request.id, "invalid_params", "unsupported track type"),
        Err(error) => return Response::failure(&request.id, error),
    };
    let Some(session) = state.sessions.get(&session_id) else {
        return dispatch(state, request);
    };
    let Some(device) = devices.get(&session.receiver_id) else {
        return failure(
            &request.id,
            "fcast_not_connected",
            "receiver is not connected",
        );
    };
    if let Err(error) = UpstreamContext::select_track(device, track_id, track_type) {
        return failure(&request.id, "fcast_track", error.to_string());
    }
    dispatch(state, request)
}

fn diagnostics_snapshot(state: &BridgeState, request: &Request) -> Response {
    let snapshot = state.warnings().fold(
        fcast_diagnostics::DiagnosticSnapshot::new(
            state.uptime_ms(),
            state.receivers.len(),
            state.sessions.len(),
        ),
        |snapshot, warning| snapshot.warning(warning),
    );
    match serde_json::to_value(snapshot) {
        Ok(value) => Response::success(&request.id, value),
        Err(error) => failure(&request.id, "diagnostics", error.to_string()),
    }
}

fn mirror_negotiate(
    _state: &mut BridgeState,
    _upstream: &UpstreamContext,
    devices: &BTreeMap<String, DeviceHandle>,
    output: &Arc<Mutex<io::Stdout>>,
    mirrors: &mut BTreeMap<String, Arc<BrowserSignaller>>,
    next_mirror_id: &mut u16,
    request: &Request,
) -> Response {
    let session_id = match request.param_string("sessionId") {
        Ok(value) => value.to_owned(),
        Err(error) => return Response::failure(&request.id, error),
    };
    let phase = request
        .params
        .get("phase")
        .and_then(Value::as_str)
        .unwrap_or("start");
    match phase {
        "start" => {
            let receiver_id = match request.param_string("receiverId") {
                Ok(value) => value,
                Err(error) => return Response::failure(&request.id, error),
            };
            let Some(device) = devices.get(receiver_id) else {
                return failure(
                    &request.id,
                    "fcast_not_connected",
                    "receiver is not connected",
                );
            };
            let numeric_id = *next_mirror_id;
            *next_mirror_id = next_mirror_id.wrapping_add(1).max(1);
            let answer_output = Arc::clone(output);
            let ready_output = Arc::clone(output);
            let answer_session_id = session_id.clone();
            let ready_session_id = session_id.clone();
            let signaller = Arc::new(BrowserSignaller::new(
                numeric_id,
                Arc::new(move |id, sdp| {
                    emit_event(
                        &answer_output,
                        "mirror.answer",
                        serde_json::json!({"sessionId": answer_session_id, "numericSessionId": id, "sdp": sdp}),
                    );
                }),
                Arc::new(move |id| {
                    emit_event(
                        &ready_output,
                        "mirror.offerReady",
                        serde_json::json!({"sessionId": ready_session_id, "numericSessionId": id}),
                    );
                }),
            ));
            if let Err(error) = UpstreamContext::start_mirroring(device, signaller.clone()) {
                return failure(&request.id, "mirror_negotiation_failed", error.to_string());
            }
            mirrors.insert(session_id.clone(), signaller);
            Response::success(
                &request.id,
                serde_json::json!({"sessionId": session_id, "state": "awaitingOffer"}),
            )
        }
        "offer" => {
            let Some(signaller) = mirrors.get(&session_id) else {
                return failure(
                    &request.id,
                    "mirror_not_started",
                    "start mirroring before sending an offer",
                );
            };
            let sdp = match request.param_string("sdp") {
                Ok(value) => value.to_owned(),
                Err(error) => return Response::failure(&request.id, error),
            };
            if let Err(error) = signaller.submit_offer(sdp) {
                return failure(&request.id, "mirror_negotiation_failed", error.to_string());
            }
            Response::success(
                &request.id,
                serde_json::json!({"sessionId": session_id, "state": "offerSent"}),
            )
        }
        "stop" => {
            mirrors.remove(&session_id);
            Response::success(
                &request.id,
                serde_json::json!({"sessionId": session_id, "state": "stopped"}),
            )
        }
        _ => failure(
            &request.id,
            "invalid_params",
            "unknown mirror negotiation phase",
        ),
    }
}

fn emit_event(output: &Arc<Mutex<io::Stdout>>, event: &str, data: Value) {
    let message = Event {
        v: fcast_bridge::PROTOCOL_VERSION,
        event: event.to_owned(),
        data,
    };
    let Ok(encoded) = serde_json::to_vec(&message) else {
        return;
    };
    let Ok(mut output) = output.lock() else {
        return;
    };
    if let Err(error) = write_frame(&mut *output, &encoded) {
        eprintln!("fcast-companion: failed to emit event: {error}");
    }
}

fn endpoint_for(
    state: &BridgeState,
    receiver_id: &str,
) -> Result<ReceiverEndpoint, (&'static str, String)> {
    let receiver = state
        .receivers
        .get(receiver_id)
        .ok_or(("receiver_not_found", receiver_id.to_owned()))?;
    let host = receiver
        .host
        .parse::<IpAddr>()
        .map_err(|_| ("invalid_receiver_host", receiver.host.clone()))?;
    let fingerprint = receiver
        .fingerprint
        .clone()
        .ok_or(("receiver_fingerprint_missing", receiver_id.to_owned()))?;
    if !receiver.trusted {
        return Err(("receiver_not_trusted", receiver_id.to_owned()));
    }
    Ok(ReceiverEndpoint {
        id: receiver.id.clone(),
        name: receiver.name.clone(),
        host,
        port: receiver.port,
        fingerprint,
    })
}

fn trust_store_from_env(get: impl Fn(&str) -> Option<PathBuf>) -> io::Result<PathBuf> {
    if let Some(path) = get("FCAST_TRUST_STORE") {
        return Ok(path);
    }
    if let Some(config) = get("XDG_CONFIG_HOME") {
        return Ok(config.join("fcast-web-sender/trust.json"));
    }
    if let Some(appdata) = get("APPDATA") {
        return Ok(appdata.join("fcast-web-sender/trust.json"));
    }
    get("HOME")
        .map(|home| home.join(".config/fcast-web-sender/trust.json"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "HOME or APPDATA is required for the trust store",
            )
        })
}

fn trust_store_path() -> io::Result<PathBuf> {
    trust_store_from_env(|key| std::env::var_os(key).map(PathBuf::from))
}

fn now_unix() -> Result<u64, std::time::SystemTimeError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn failure(id: &str, code: impl Into<String>, message: impl Into<String>) -> Response {
    Response::failure_with(
        id,
        ResponseError {
            code: code.into(),
            message: message.into(),
            recoverable: false,
            suggested_next_mode: None,
            details: None,
        },
    )
}

fn malformed_request_response(error: ProtocolError) -> Response {
    failure("invalid-request", "invalid_request", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcast_receiver_trust::fingerprint_spki;

    fn request(method: &str, params: serde_json::Value) -> Request {
        Request {
            v: fcast_bridge::PROTOCOL_VERSION,
            id: "test".into(),
            method: method.into(),
            params: params.as_object().unwrap().clone(),
        }
    }

    fn state_with_receiver() -> (BridgeState, String) {
        let fingerprint = fingerprint_spki(b"receiver-key").to_string();
        let mut state = BridgeState::new();
        state.receivers.insert(
            "receiver-1".into(),
            ReceiverSummary {
                id: "receiver-1".into(),
                name: "Receiver".into(),
                host: "192.0.2.1".into(),
                port: 46899,
                fingerprint: Some(fingerprint.clone()),
                connection_state: ConnectionState::Discovered,
                trusted: true,
                ttl_seconds: Some(60),
            },
        );
        (state, fingerprint)
    }

    #[test]
    fn trust_rejects_a_fingerprint_that_was_not_discovered() {
        let (mut state, _) = state_with_receiver();
        state.receivers.get_mut("receiver-1").unwrap().trusted = false;
        let other = fingerprint_spki(b"attacker-key");
        let response = trust_receiver(
            &mut state,
            &mut TrustStore::in_memory(),
            &request(
                "receiver.trust",
                serde_json::json!({
                    "receiverId": "receiver-1",
                    "fingerprint": other.as_str(),
                }),
            ),
        );
        assert_eq!(response.error().unwrap().code, "fingerprint_mismatch");
        assert!(!state.receivers["receiver-1"].trusted);
    }

    #[test]
    fn credential_lease_is_https_origin_bound_and_single_use() {
        let (state, fingerprint) = state_with_receiver();
        let mut leases = CredentialLeases::default();
        let response = create_credential_lease(
            &state,
            &mut leases,
            &request(
                "credentialLease.create",
                serde_json::json!({
                    "receiverId": "receiver-1",
                    "url": "https://media.example/master.m3u8",
                    "headers": {"Authorization": "Bearer secret"},
                }),
            ),
        );
        let lease_id = response
            .result()
            .and_then(|value| value.get("credentialLeaseId"))
            .and_then(Value::as_str)
            .unwrap()
            .to_owned();
        assert!(
            leases
                .claim(
                    &lease_id,
                    "receiver-1",
                    Some(&fingerprint),
                    &Url::parse("https://other.example/video").unwrap(),
                )
                .is_err()
        );
        let headers = leases
            .claim(
                &lease_id,
                "receiver-1",
                Some(&fingerprint),
                &Url::parse("https://media.example/video").unwrap(),
            )
            .unwrap();
        assert_eq!(headers[0].0, "Authorization");
        assert!(
            leases
                .claim(
                    &lease_id,
                    "receiver-1",
                    Some(&fingerprint),
                    &Url::parse("https://media.example/video").unwrap(),
                )
                .is_err()
        );
    }

    #[test]
    fn trust_store_uses_appdata_when_home_is_absent() {
        let appdata = PathBuf::from("AppData/Roaming");
        let path = trust_store_from_env(|key| match key {
            "APPDATA" => Some(appdata.clone()),
            _ => None,
        })
        .unwrap();
        assert_eq!(path, appdata.join("fcast-web-sender/trust.json"));
    }
}
