//! `x-obscura-context` gives each CDP connection its own browser identity.
//!
//! Callers that drive several accounts through one Obscura process rely on
//! three properties, and only an end-to-end CDP test can show them, because the
//! seed travels from an HTTP upgrade header, through connection setup, into the
//! per-page V8 realm:
//!
//! 1. a header that does not describe a supported identity is refused at the
//!    handshake with `400 invalid browser context`, never silently downgraded
//!    to the shared default identity;
//! 2. the same header produces the same fingerprint on a later connection, so
//!    an account keeps its identity across reconnects;
//! 3. different headers produce different fingerprints, so two accounts sharing
//!    one process are not fingerprinted as the same browser.
//!
//! (1) is the one that matters most: a rejected header that fell back to the
//! default identity would leave every caller believing it had per-account
//! isolation while sharing one fingerprint, and nothing else here would notice.
//!
//! Run with `cargo nextest run -p obscura-cdp -E 'test(connection_identity)'`.

use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Fingerprint surfaces that `_getFp()` derives from the seed alone. `screen`
/// picks from a fixed pool, so a pair of seeds could collide there;
/// `baseLatency` is continuous, which is what makes the comparison decisive.
const FINGERPRINT: &str = "(() => { const audio = new AudioContext(); \
    return [screen.width, screen.height, audio.sampleRate, audio.baseLatency]; })()";

async fn pick_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn context_header(seed: &str) -> String {
    URL_SAFE_NO_PAD.encode(
        json!({
            "version": 1,
            "fingerprintSeed": seed,
            "browserProfile": "chrome_145",
            "operatingSystem": "windows",
        })
        .to_string(),
    )
}

/// Open a connection carrying `header`, create a page on it, and read the
/// fingerprint the realm ended up with. Every step is bounded so a server that
/// stops answering fails the test instead of hanging the run.
async fn fingerprint_over_connection(ws_port: u16, header: Option<&str>) -> Result<Value, String> {
    let mut request = format!("ws://127.0.0.1:{ws_port}/devtools/browser")
        .into_client_request()
        .map_err(|e| e.to_string())?;
    if let Some(header) = header {
        request.headers_mut().insert(
            "x-obscura-context",
            HeaderValue::from_str(header).map_err(|e| e.to_string())?,
        );
    }
    let (mut ws, _) = connect_async(request).await.map_err(|e| e.to_string())?;

    ws.send(Message::Text(
        json!({
            "id": 1,
            "method": "Target.createTarget",
            "params": {"url": "data:text/html,<title>identity</title>"},
        })
        .to_string()
        .into(),
    ))
    .await
    .map_err(|e| e.to_string())?;

    let session = read_until(&mut ws, |value| {
        value
            .get("params")
            .and_then(|params| params.get("sessionId"))
            .and_then(Value::as_str)
            .map(str::to_string)
    })
    .await?;

    ws.send(Message::Text(
        json!({
            "id": 2,
            "method": "Runtime.evaluate",
            "sessionId": session,
            "params": {"expression": FINGERPRINT, "returnByValue": true},
        })
        .to_string()
        .into(),
    ))
    .await
    .map_err(|e| e.to_string())?;

    let fingerprint = read_until(&mut ws, |value| {
        if value.get("id").and_then(Value::as_u64) != Some(2) {
            return None;
        }
        Some(
            value
                .pointer("/result/result/value")
                .cloned()
                .unwrap_or(Value::Null),
        )
    })
    .await?;

    let _ = ws.close(None).await;
    if fingerprint.as_array().is_none_or(|values| values.len() != 4) {
        return Err(format!("unusable fingerprint: {fingerprint}"));
    }
    Ok(fingerprint)
}

async fn read_until<T>(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut select: impl FnMut(Value) -> Option<T>,
) -> Result<T, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .ok_or("timed out waiting for a matching CDP message")?;
        let message = tokio::time::timeout(remaining, ws.next())
            .await
            .map_err(|_| "timed out waiting for a matching CDP message".to_string())?
            .ok_or("connection closed")?
            .map_err(|e| e.to_string())?;
        let Message::Text(text) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if let Some(selected) = select(value) {
            return Ok(selected);
        }
    }
}

/// Hand-rolled handshake: a refusal is an HTTP response, and a websocket client
/// would collapse it into an opaque error rather than the status and reason we
/// need to assert on.
async fn handshake_response(ws_port: u16, header: &str) -> String {
    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", ws_port))
        .await
        .expect("connect");
    let request = format!(
        "GET /devtools/browser HTTP/1.1\r\nHost: 127.0.0.1:{ws_port}\r\n\
         Upgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\
         X-Obscura-Context: {header}\r\n\r\n"
    );
    socket.write_all(request.as_bytes()).await.expect("write");
    let mut buffer = vec![0u8; 1024];
    let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
        .await
        .expect("handshake response timed out")
        .expect("read");
    String::from_utf8_lossy(&buffer[..read]).to_string()
}

#[test]
fn connection_identity_is_validated_stable_and_isolated() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&runtime, async move {
        let ws_port = pick_port().await;
        tokio::task::spawn_local(async move {
            let _ = obscura_cdp::start_with_serve_options_and_limit(
                ws_port,
                "127.0.0.1",
                None,
                false,
                None,
                false,
                None,
                true,
                8,
            )
            .await;
        });
        tokio::time::sleep(Duration::from_millis(300)).await;

        // 1. Identities the build cannot honour are refused at the handshake.
        for (case, header) in [
            ("not base64url", "!!!not-base64!!!"),
            ("not JSON", &URL_SAFE_NO_PAD.encode("plain text")),
            (
                "unsupported profile",
                &URL_SAFE_NO_PAD.encode(
                    json!({
                        "version": 1,
                        "fingerprintSeed": "seed",
                        "browserProfile": "firefox_1",
                        "operatingSystem": "windows",
                    })
                    .to_string(),
                ),
            ),
            (
                "unsupported version",
                &URL_SAFE_NO_PAD.encode(
                    json!({
                        "version": 2,
                        "fingerprintSeed": "seed",
                        "browserProfile": "chrome_145",
                        "operatingSystem": "windows",
                    })
                    .to_string(),
                ),
            ),
        ] {
            let response = handshake_response(ws_port, header).await;
            assert!(
                response.starts_with("HTTP/1.1 400"),
                "{case} must be refused, not accepted under the default identity; got {:?}",
                response.lines().next()
            );
            assert!(
                response.contains("invalid browser context"),
                "{case} refusal must name the reason: {response:?}"
            );
        }

        // 2. The same identity survives a reconnect.
        let header = context_header("account-one");
        let first = fingerprint_over_connection(ws_port, Some(&header))
            .await
            .expect("first identified connection");
        let reconnected = fingerprint_over_connection(ws_port, Some(&header))
            .await
            .expect("reconnected identified connection");
        assert_eq!(
            first, reconnected,
            "one identity must fingerprint the same way across reconnects"
        );

        // 3. A different identity is a different browser.
        let other = fingerprint_over_connection(ws_port, Some(&context_header("account-two")))
            .await
            .expect("second identified connection");
        assert_ne!(
            first, other,
            "two identities sharing a process must not share a fingerprint"
        );

        // 4. Connections without a header keep working on the default identity.
        fingerprint_over_connection(ws_port, None)
            .await
            .expect("unidentified connections must still be served");
    });
}
