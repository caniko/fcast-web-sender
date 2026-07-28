use fcast_client::FCAST_V4;
use serde_json::json;
use std::io::{self, BufRead, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|argument| argument == "--version") {
        println!(
            "fcast-mock-receiver {} (FCast v{FCAST_V4})",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    // The fixture receiver is line-oriented so integration tests can drive it
    // without binding a real receiver port. It still validates that commands
    // are JSON control messages and reports deterministic acknowledgements.
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        let command: serde_json::Value = serde_json::from_str(&line)?;
        serde_json::to_writer(
            &mut stdout,
            &json!({
                "ok": true,
                "protocolVersion": FCAST_V4,
                "command": command,
            }),
        )?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}
