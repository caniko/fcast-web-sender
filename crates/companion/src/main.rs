use fcast_bridge::{
    BridgeState, ConnectionState, Event, ProtocolError, ReceiverSummary, Request, Response,
    ResponseError, dispatch, read_frame, write_frame,
};
use fcast_client::{
    LoadRequest, ReceiverEndpoint,
    upstream::{BrowserSignaller, DeviceHandle, UpstreamContext, UpstreamEvent},
};
use fcast_discovery::{MdnsBrowser, SERVICE_NAME};
use fcast_media_fetcher::{FetchedFile, MediaFetcher, UrlPolicy};
use fcast_receiver_trust::{TrustStore, normalize_fingerprint};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{self, stdin, stdout};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        "session.load" => load_session(
            context.state,
            context.upstream,
            context.devices,
            context.runtime,
            context.media_fetcher,
            context.files,
            request,
        ),
        "session.control" => control_session(context.state, context.devices, request),
        "session.selectTrack" => select_track(context.state, context.devices, request),
        "session.close" => close_session(context.state, context.files, request),
        "diagnostics.snapshot" => diagnostics_snapshot(context.state, request),
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

fn discover_receivers(state: &mut BridgeState, trust_store: &TrustStore) {
    let browser = match MdnsBrowser::bind(Duration::from_millis(250)) {
        Ok(browser) => browser,
        Err(error) => {
            eprintln!("fcast-companion: discovery socket unavailable: {error}");
            return;
        }
    };
    let records = match browser.discover(SERVICE_NAME) {
        Ok(records) => records,
        Err(error) => {
            eprintln!("fcast-companion: discovery failed: {error}");
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
            normalize_fingerprint(&format!("sha256/{raw}")).ok()
        });
        let trusted = fingerprint
            .as_deref()
            .is_some_and(|fingerprint| trust_store.is_trusted(fingerprint));
        state.receivers.insert(
            record.id.clone(),
            ReceiverSummary {
                id: record.id,
                name,
                host: host.to_string(),
                port: record.port,
                fingerprint,
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
    let fingerprint = match normalize_fingerprint(raw_fingerprint) {
        Ok(value) => value,
        Err(error) => return failure(&request.id, "invalid_fingerprint", error.to_string()),
    };
    let label = request
        .params
        .get("label")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if let Err(error) = trust_store.trust(&fingerprint, label, now_unix()) {
        return failure(&request.id, "trust_store", error.to_string());
    }
    let mut normalized = request.clone();
    normalized
        .params
        .insert("fingerprint".into(), Value::String(fingerprint));
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
    let response = dispatch(state, request);
    if !response.ok {
        return response;
    }
    if let Some(fingerprint) = fingerprint
        && let Err(error) = trust_store.forget(&fingerprint)
    {
        return failure(&request.id, "trust_store", error.to_string());
    }
    response
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
        Err(error) => return failure(&request.id, "fcast_connect", error),
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
    if let Some(device) = devices.remove(&receiver_id)
        && let Err(error) = device.disconnect()
    {
        return failure(&request.id, "fcast_disconnect", error.to_string());
    }
    dispatch(state, request)
}

fn load_session(
    state: &mut BridgeState,
    upstream: &UpstreamContext,
    devices: &BTreeMap<String, DeviceHandle>,
    runtime: &tokio::runtime::Runtime,
    media_fetcher: &MediaFetcher,
    files: &mut BTreeMap<String, Vec<FetchedFile>>,
    request: &Request,
) -> Response {
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
            return failure(&request.id, "fcast_load", error);
        }
        return dispatch(state, request);
    }

    let headers = match request_headers(request) {
        Ok(headers) => headers,
        Err(error) => return failure(&request.id, "media_headers_invalid", error),
    };
    let allow_sensitive = request
        .params
        .get("credentialLease")
        .and_then(Value::as_bool)
        .unwrap_or(false);
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
        return failure(&request.id, "fcast_load", error);
    }
    let response = dispatch(state, request);
    if response.ok
        && let Some(session_id) = response
            .result
            .as_ref()
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
    if response.ok {
        files.remove(&session_id);
    }
    response
}

fn request_headers(request: &Request) -> Result<Vec<(String, String)>, String> {
    let Some(value) = request.params.get("headers") else {
        return Ok(Vec::new());
    };
    let Some(object) = value.as_object() else {
        return Err("'headers' must be an object".into());
    };
    if object.len() > 16 {
        return Err("at most 16 media headers are allowed".into());
    }
    object
        .iter()
        .map(|(name, value)| {
            let value = value
                .as_str()
                .ok_or_else(|| format!("header '{name}' must be a string"))?;
            Ok((name.clone(), value.to_owned()))
        })
        .collect()
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
    let action = match request.param_string("action") {
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
    let result = fcast_client::upstream::UpstreamContext::control(
        device,
        action,
        request.params.get("positionMs").and_then(Value::as_f64),
        request.params.get("level").and_then(Value::as_f64),
        request.params.get("speed").and_then(Value::as_f64),
    );
    if let Err(error) = result {
        return failure(&request.id, "fcast_control", error);
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
    if let Err(error) = UpstreamContext::select_track(device, track_id, track_type) {
        return failure(&request.id, "fcast_track", error);
    }
    dispatch(state, request)
}

fn diagnostics_snapshot(state: &BridgeState, request: &Request) -> Response {
    let snapshot = fcast_diagnostics::DiagnosticSnapshot::new(
        state.uptime_ms(),
        state.receivers.len(),
        state.sessions.len(),
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
                return failure(&request.id, "mirror_negotiation_failed", error);
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
                return failure(&request.id, "mirror_negotiation_failed", error);
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

fn trust_store_path() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("FCAST_TRUST_STORE") {
        return Ok(PathBuf::from(path));
    }
    if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config).join("fcast-web-sender/trust.json"));
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".config/fcast-web-sender/trust.json"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "HOME is required for the trust store",
            )
        })
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn failure(id: &str, code: impl Into<String>, message: impl Into<String>) -> Response {
    Response {
        v: fcast_bridge::PROTOCOL_VERSION,
        id: id.into(),
        ok: false,
        result: None,
        error: Some(ResponseError {
            code: code.into(),
            message: message.into(),
            recoverable: false,
            suggested_next_mode: None,
            details: None,
        }),
    }
}

fn malformed_request_response(error: ProtocolError) -> Response {
    failure("invalid-request", "invalid_request", error.to_string())
}
