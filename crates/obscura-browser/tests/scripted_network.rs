use std::sync::Arc;
use std::time::Duration;

use obscura_browser::{BrowserContext, Page};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// One server task, three requests, and a deadline; no external network or workers.
async fn scripted_responses_are_observable(stealth: bool) {
    std::env::set_var("OBSCURA_ALLOW_PRIVATE_NETWORK", "1");
    tokio::time::timeout(Duration::from_secs(15), async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 4096];
                let size = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..size]);
                let body = if request.starts_with("POST /fetch ") {
                    r#"{"transport":"fetch"}"#
                } else if request.starts_with("GET /xhr ") {
                    r#"{"transport":"xhr"}"#
                } else {
                    "<!doctype html><title>network fixture</title>"
                };
                socket.write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ).as_bytes()).await.unwrap();
            }
        });
        let context = Arc::new(BrowserContext::with_storage_and_network(
            "scripted-network".into(), None, stealth, None, None, true,
        ));
        let mut page = Page::new("scripted-network".into(), context);
        page.navigate(&base).await.unwrap();
        page.network_events.clear();
        let result = page.evaluate_for_cdp(
            r#"(async () => {
                const response = await fetch('/fetch', {
                    method: 'POST',
                    headers: {'content-type': 'application/x-www-form-urlencoded', 'x-fixture': 'timeline'},
                    body: 'cursor=next&count=12',
                });
                const xhrStatus = await new Promise(resolve => {
                    const xhr = new XMLHttpRequest();
                    xhr.open('GET', '/xhr');
                    xhr.onloadend = () => resolve(xhr.status);
                    xhr.send();
                });
                return [response.status, xhrStatus];
            })()"#, true, true,
        ).await;
        assert_eq!(result.value, Some(serde_json::json!([200, 200])));
        page.sync_js_network_events();
        assert_eq!(page.network_events.len(), 2, "fetch and XHR must both reach CDP");
        for (event, transport) in page.network_events.iter().zip(["fetch", "xhr"]) {
            assert_eq!(event.url, format!("{base}/{transport}"));
            assert_eq!(event.status, 200);
            if transport == "fetch" {
                assert_eq!(event.method, "POST");
                assert_eq!(event.post_data.as_deref(), Some("cursor=next&count=12"));
                assert_eq!(event.headers.get("x-fixture").map(String::as_str), Some("timeline"));
            } else {
                assert_eq!(event.method, "GET");
                assert_eq!(event.post_data, None);
            }
            let body = page.get_response_body(&event.request_id).expect("CDP response body");
            assert!(!body.base64_encoded);
            assert_eq!(body.body, format!(r#"{{"transport":"{transport}"}}"#));
        }
        assert_ne!(page.network_events[0].request_id, page.network_events[1].request_id);
        page.network_events.clear();
        page.sync_js_network_events();
        assert!(page.network_events.is_empty(), "events must be drained once");
        server.await.unwrap();
    }).await.expect("scripted network deadline");
}

#[tokio::test(flavor = "current_thread")]
async fn scripted_network_responses_plain() {
    scripted_responses_are_observable(false).await;
}

#[cfg(feature = "stealth")]
#[tokio::test(flavor = "current_thread")]
async fn scripted_network_responses_stealth() {
    scripted_responses_are_observable(true).await;
}
