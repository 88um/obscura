use std::collections::HashMap;
use base64::Engine as _;

use crate::ops::{JsNetworkEvent, SharedState, StoredNetworkResponseBody};

pub(crate) struct ScriptedResponse<'a> {
    pub url: &'a str,
    pub method: &'a str,
    pub request_headers: &'a HashMap<String, String>,
    pub request_body: &'a [u8],
    pub status: u16,
    pub headers: &'a HashMap<String, String>,
    pub body: &'a [u8],
}

/// Store the body and event together so either transport produces a CDP request
/// ID that Network.getResponseBody can resolve. Interception keeps its own ID.
pub(crate) fn record_scripted_response(
    state: &SharedState,
    request_id: Option<String>,
    response: ScriptedResponse<'_>,
) -> String {
    let mut state = state.borrow_mut();
    let request_id = request_id.unwrap_or_else(|| {
        state.network_response_body_counter += 1;
        format!("fetch-{}", state.network_response_body_counter)
    });
    let max_entries = std::env::var("OBSCURA_NETWORK_BODY_BUFFER_ENTRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128);
    let max_bytes = std::env::var("OBSCURA_NETWORK_BODY_BUFFER_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2 * 1024 * 1024);
    if max_entries > 0 && max_bytes > 0 && response.body.len() <= max_bytes {
        state.network_response_bodies.insert(
            request_id.clone(),
            StoredNetworkResponseBody {
                body: match std::str::from_utf8(response.body) {
                    Ok(text) => text.to_owned(),
                    Err(_) => base64::engine::general_purpose::STANDARD.encode(response.body),
                },
                base64_encoded: std::str::from_utf8(response.body).is_err(),
            },
        );
        state
            .network_response_body_order
            .push_back(request_id.clone());
        while state.network_response_body_order.len() > max_entries {
            if let Some(oldest) = state.network_response_body_order.pop_front() {
                state.network_response_bodies.remove(&oldest);
            }
        }
    }
    state.js_network_events.push(JsNetworkEvent {
        request_id: request_id.clone(),
        url: response.url.to_string(),
        method: response.method.to_string(),
        status: response.status,
        request_headers: response.request_headers.clone(),
        post_data_bytes: (!response.request_body.is_empty()).then(|| response.request_body.to_vec()),
        post_data: (!response.request_body.is_empty()).then(|| String::from_utf8_lossy(response.request_body).into_owned()),
        response_headers: response.headers.clone(),
        body_size: response.body.len(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64(),
    });
    const MAX_JS_NETWORK_EVENTS: usize = 4096;
    if state.js_network_events.len() > MAX_JS_NETWORK_EVENTS {
        let overflow = state.js_network_events.len() - MAX_JS_NETWORK_EVENTS;
        state.js_network_events.drain(0..overflow);
    }
    request_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intercepted_binary_response_keeps_id_and_exact_request_and_response_bytes() {
        let state = std::rc::Rc::new(std::cell::RefCell::new(crate::ops::ObscuraState::new()));
        let headers = HashMap::new();
        let id = record_scripted_response(&state, Some("intercept-7".into()), ScriptedResponse {
            url: "https://example.test/upload",
            method: "POST",
            request_headers: &headers,
            request_body: &[0, 255, 128],
            status: 200,
            headers: &headers,
            body: &[255, 0, 128],
        });
        assert_eq!(id, "intercept-7");
        let state = state.borrow();
        let body = &state.network_response_bodies[&id];
        assert!(body.base64_encoded);
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&body.body).unwrap(), [255, 0, 128]);
        assert_eq!(state.js_network_events[0].request_id, id);
        assert_eq!(state.js_network_events[0].post_data_bytes.as_deref(), Some([0, 255, 128].as_slice()));
    }
}
