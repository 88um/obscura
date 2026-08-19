use base64::{engine::general_purpose, Engine as _};
use obscura_browser::BrowserIdentity;
use serde::Deserialize;
use url::Url;

const CONTEXT_HEADER: &str = "x-obscura-context";
const MAX_HEADER_VALUE_BYTES: usize = 4_096;
const MAX_DECODED_CONTEXT_BYTES: usize = 4_096;
const MAX_SEED_BYTES: usize = 256;
const MAX_PROXY_BYTES: usize = 2_048;
const MAX_TIMEZONE_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InvalidContext;

impl std::fmt::Display for InvalidContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid browser context")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireContext {
    version: u32,
    #[serde(rename = "fingerprintSeed")]
    fingerprint_seed: String,
    #[serde(rename = "browserProfile")]
    browser_profile: String,
    #[serde(rename = "operatingSystem")]
    operating_system: String,
    #[serde(rename = "proxyUrl", default)]
    proxy_url: Option<String>,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    geolocation: Option<WireGeolocation>,
}

#[derive(Deserialize)]
struct WireGeolocation {
    latitude: f64,
    longitude: f64,
}

/// Parse and validate the bounded HTTP upgrade request without retaining or
/// formatting its raw header value in any error path.
pub(crate) fn parse_upgrade_context(
    request: &[u8],
    stealth: bool,
) -> Result<Option<BrowserIdentity>, InvalidContext> {
    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|offset| offset + 4)
        .ok_or(InvalidContext)?;
    if header_end > MAX_HEADER_VALUE_BYTES * 2 {
        return Err(InvalidContext);
    }

    let request = std::str::from_utf8(&request[..header_end]).map_err(|_| InvalidContext)?;
    let mut lines = request.split("\r\n");
    let request_line = lines.next().ok_or(InvalidContext)?;
    if !request_line.starts_with("GET ") {
        return Err(InvalidContext);
    }

    let mut encoded = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').ok_or(InvalidContext)?;
        if name.eq_ignore_ascii_case(CONTEXT_HEADER) {
            if encoded.is_some() {
                return Err(InvalidContext);
            }
            let value = value.trim();
            if value.is_empty() || value.len() > MAX_HEADER_VALUE_BYTES {
                return Err(InvalidContext);
            }
            encoded = Some(value);
        }
    }

    let Some(encoded) = encoded else {
        return Ok(None);
    };
    let decoded = general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| general_purpose::URL_SAFE.decode(encoded))
        .map_err(|_| InvalidContext)?;
    if decoded.is_empty() || decoded.len() > MAX_DECODED_CONTEXT_BYTES {
        return Err(InvalidContext);
    }
    let wire: WireContext = serde_json::from_slice(&decoded).map_err(|_| InvalidContext)?;
    validate_wire_context(wire, stealth)
}

fn validate_wire_context(
    wire: WireContext,
    stealth: bool,
) -> Result<Option<BrowserIdentity>, InvalidContext> {
    if wire.version != 1
        || wire.browser_profile != "chrome_145"
        || wire.operating_system != "windows"
        || wire.fingerprint_seed.is_empty()
        || wire.fingerprint_seed.len() > MAX_SEED_BYTES
        || !wire.fingerprint_seed.is_ascii()
        || wire
            .fingerprint_seed
            .bytes()
            .any(|byte| byte.is_ascii_control())
    {
        return Err(InvalidContext);
    }

    // The only currently supported stealth transport is Chrome 145 on
    // Windows. Keep the explicit check here so future profile additions do
    // not accidentally claim a matching TLS identity before they exist.
    if stealth && (wire.browser_profile != "chrome_145" || wire.operating_system != "windows") {
        return Err(InvalidContext);
    }

    if let Some(proxy) = wire.proxy_url.as_deref() {
        if !valid_proxy(proxy) {
            return Err(InvalidContext);
        }
    }

    if let Some(timezone) = wire.timezone.as_deref() {
        if timezone.is_empty()
            || timezone.len() > MAX_TIMEZONE_BYTES
            || !timezone.is_ascii()
            || timezone.bytes().any(|byte| byte.is_ascii_control())
            || std::env::var("TZ").ok().as_deref() != Some(timezone)
        {
            return Err(InvalidContext);
        }
    }

    let geolocation = match wire.geolocation {
        Some(geo)
            if geo.latitude.is_finite()
                && geo.longitude.is_finite()
                && (-90.0..=90.0).contains(&geo.latitude)
                && (-180.0..=180.0).contains(&geo.longitude) =>
        {
            Some((geo.latitude, geo.longitude))
        }
        Some(_) => return Err(InvalidContext),
        None => None,
    };

    Ok(Some(BrowserIdentity {
        fingerprint_seed: wire.fingerprint_seed,
        browser_profile: wire.browser_profile,
        operating_system: wire.operating_system,
        proxy_url: wire.proxy_url,
        timezone: wire.timezone,
        geolocation,
    }))
}

fn valid_proxy(proxy: &str) -> bool {
    if proxy.is_empty()
        || proxy.len() > MAX_PROXY_BYTES
        || !proxy.is_ascii()
        || proxy.bytes().any(|byte| byte.is_ascii_control())
    {
        return false;
    }
    let Ok(url) = Url::parse(proxy) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    fn request(payload: &str) -> Vec<u8> {
        let encoded = URL_SAFE_NO_PAD.encode(payload);
        format!(
            "GET /devtools/browser HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nX-Obscura-Context: {encoded}\r\n\r\n"
        )
        .into_bytes()
    }

    fn valid_payload(seed: &str) -> String {
        format!(
            r#"{{"version":1,"fingerprintSeed":"{seed}","browserProfile":"chrome_145","operatingSystem":"windows"}}"#
        )
    }

    #[test]
    fn absent_header_keeps_legacy_context() {
        let result = parse_upgrade_context(
            b"GET /devtools/browser HTTP/1.1\r\nHost: localhost\r\n\r\n",
            false,
        )
        .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn valid_header_is_parsed_without_exposing_raw_identity() {
        let secret = "proxy-password-and-seed";
        let result = parse_upgrade_context(&request(&valid_payload(secret)), false).unwrap();
        let identity = result.unwrap();
        assert_eq!(identity.fingerprint_seed, secret);
        assert!(!format!("{InvalidContext}").contains(secret));
    }

    #[test]
    fn malformed_and_oversized_headers_are_generic() {
        let malformed = request("not-json");
        let error = match parse_upgrade_context(&malformed, false) {
            Err(error) => error,
            Ok(_) => panic!("malformed context accepted"),
        };
        assert_eq!(error.to_string(), "invalid browser context");

        let oversized = format!(
            "GET /devtools/browser HTTP/1.1\r\nX-Obscura-Context: {}\r\n\r\n",
            "A".repeat(MAX_HEADER_VALUE_BYTES + 1)
        );
        assert!(parse_upgrade_context(oversized.as_bytes(), false).is_err());
    }

    #[test]
    fn timezone_must_match_process_timezone() {
        let payload = valid_payload("seed").replace(
            "\"operatingSystem\":\"windows\"",
            "\"operatingSystem\":\"windows\",\"timezone\":\"not-process-zone\"",
        );
        assert!(parse_upgrade_context(&request(&payload), false).is_err());
    }
}
