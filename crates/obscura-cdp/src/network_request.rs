use base64::Engine;
use obscura_browser::NetworkEvent;
use serde_json::{json, Value};

/// Encode one completed request for both navigation and runtime CDP events.
/// Playwright reads postDataEntries; clients using postData still receive it.
pub(crate) fn request_payload(event: &NetworkEvent) -> Value {
    let mut request = json!({
        "url": event.url,
        "method": event.method,
        "headers": event.headers,
        "hasPostData": event.post_data.is_some(),
    });
    if let Some(body) = &event.post_data {
        request["postData"] = json!(body);
        request["postDataEntries"] = json!([{
            "bytes": base64::engine::general_purpose::STANDARD.encode(body.as_bytes()),
        }]);
    }
    request
}
