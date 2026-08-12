use fcast_bridge::{BridgeState, Request, dispatch, read_frame, write_frame};
use serde_json::json;
use std::io::Cursor;

#[test]
fn bridge_hello_is_a_control_only_response() {
    let request =
        Request::parse(br#"{"v":1,"id":"integration-1","method":"bridge.hello","params":{}}"#)
            .unwrap();
    let response = dispatch(&mut BridgeState::new(), &request);
    assert!(response.is_ok());
    assert_eq!(response.version(), 1);
    let result = response.result().unwrap();
    assert_eq!(result["protocolVersion"], json!(1));
    assert!(
        result["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value.as_str().unwrap() != "mediaBytes")
    );
}

#[test]
fn native_frame_round_trip_preserves_json_without_media_payloads() {
    let body = br#"{"v":1,"id":"1","method":"diagnostics.snapshot","params":{}}"#;
    let mut framed = Vec::new();
    write_frame(&mut framed, body).unwrap();
    assert_eq!(
        read_frame(&mut Cursor::new(framed)).unwrap(),
        Some(body.to_vec())
    );
}
