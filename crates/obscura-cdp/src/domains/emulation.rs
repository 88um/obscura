use serde_json::{json, Value};

use crate::dispatch::CdpContext;

pub async fn handle(
    method: &str,
    params: &Value,
    ctx: &mut CdpContext,
    session_id: &Option<String>,
) -> Result<Value, String> {
    match method {
        "setDeviceMetricsOverride" => {
            let width = params
                .get("width")
                .and_then(|v| v.as_u64())
                .unwrap_or(1920)
                .min(u32::MAX as u64) as u32;
            let height = params
                .get("height")
                .and_then(|v| v.as_u64())
                .unwrap_or(1000)
                .min(u32::MAX as u64) as u32;
            let device_scale_factor = params
                .get("deviceScaleFactor")
                .and_then(|v| v.as_f64())
                .unwrap_or(2.0);

            if let Some(page) = ctx.get_session_page_mut(session_id) {
                page.set_viewport_metrics(width, height, device_scale_factor);
            }

            Ok(json!({}))
        }
        "clearDeviceMetricsOverride" => {
            if let Some(page) = ctx.get_session_page_mut(session_id) {
                page.set_viewport_metrics(1920, 1000, 2.0);
            }
            Ok(json!({}))
        }
        "setUserAgentOverride" => {
            if let Some(page) = ctx.get_session_page_mut(session_id) {
                if let Some(ua) = params.get("userAgent").and_then(|v| v.as_str()) {
                    page.http_client.set_user_agent(ua).await;
                }

                if let Some(accept_language) = params.get("acceptLanguage").and_then(|v| v.as_str())
                {
                    let mut headers = page.http_client.extra_headers.write().await;
                    headers.insert("accept-language".to_string(), accept_language.to_string());
                }

                if let Some(metadata) = params.get("userAgentMetadata").and_then(|v| v.as_object())
                {
                    if let Some(platform) = metadata.get("platform").and_then(|v| v.as_str()) {
                        let mut headers = page.http_client.extra_headers.write().await;
                        headers.insert(
                            "sec-ch-ua-platform".to_string(),
                            format!("\"{}\"", platform),
                        );
                    }
                    if let Some(platform_version) =
                        metadata.get("platformVersion").and_then(|v| v.as_str())
                    {
                        let mut headers = page.http_client.extra_headers.write().await;
                        headers.insert(
                            "sec-ch-ua-platform-version".to_string(),
                            format!("\"{}\"", platform_version),
                        );
                    }
                }

                page.set_user_agent_override(params.clone());
            }

            Ok(json!({}))
        }
        "setTouchEmulationEnabled"
        | "setEmulatedMedia"
        | "setTimezoneOverride"
        | "setLocaleOverride"
        | "setCPUThrottlingRate"
        | "setScriptExecutionDisabled"
        | "setFocusEmulationEnabled"
        | "setScrollbarsHidden"
        | "setDefaultBackgroundColorOverride" => Ok(json!({})),
        _ => Err(format!("Unknown Emulation method: {}", method)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_user_agent_override_accepts_playwright_context_params() {
        let mut ctx = CdpContext::new();
        let page_id = ctx.create_page();
        let session_id = Some("session-1".to_string());
        ctx.sessions.insert("session-1".to_string(), page_id);

        let result = handle(
            "setUserAgentOverride",
            &json!({
                "userAgent": "Playwright-Test-UA/1.0",
                "acceptLanguage": "en-US",
                "platform": "MacIntel",
                "userAgentMetadata": {
                    "platform": "macOS",
                    "platformVersion": "14.0.0"
                }
            }),
            &mut ctx,
            &session_id,
        )
        .await
        .expect("setUserAgentOverride should be accepted");

        assert_eq!(result, json!({}));

        let page = ctx
            .get_session_page(&session_id)
            .expect("session should still point at the page");
        assert_eq!(
            page.http_client.user_agent.read().await.as_str(),
            "Playwright-Test-UA/1.0"
        );
        let headers = page.http_client.extra_headers.read().await;
        assert_eq!(
            headers.get("accept-language").map(String::as_str),
            Some("en-US")
        );
        assert_eq!(
            headers.get("sec-ch-ua-platform").map(String::as_str),
            Some("\"macOS\"")
        );
        assert!(page.user_agent_override.is_some());
    }
}
