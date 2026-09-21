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
        "hasPostData": event.post_data.is_some() || event.post_data_bytes.is_some(),
    });
    if let Some(body) = &event.post_data {
        request["postData"] = json!(body);
    }
    if let Some(body) = event.post_data_bytes.as_deref()
        .or_else(|| event.post_data.as_deref().map(str::as_bytes))
    {
        request["postDataEntries"] = json!([{
            "bytes": base64::engine::general_purpose::STANDARD.encode(body),
        }]);
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playwright_post_entries_preserve_binary_upload_bytes() {
        let event = NetworkEvent {
            request_id: "fetch-1".into(),
            url: "https://example.test/upload".into(),
            method: "POST".into(),
            headers: Default::default(),
            post_data: Some(String::from_utf8_lossy(&[0, 255, 128]).into_owned()),
            post_data_bytes: Some(vec![0, 255, 128]),
            resource_type: "Fetch".into(),
            status: 200,
            response_headers: Default::default(),
            body_size: 0,
            timestamp: 0.0,
        };
        let request = request_payload(&event);
        assert_eq!(request["hasPostData"], true);
        assert_eq!(request["postDataEntries"][0]["bytes"], "AP+A");
    }
}
