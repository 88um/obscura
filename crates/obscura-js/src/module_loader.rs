use std::pin::Pin;

use deno_core::error::ModuleLoaderError;
use deno_core::ModuleLoadResponse;
use deno_core::ModuleLoader;
use deno_core::ModuleSource;
use deno_core::ModuleSourceCode;
use deno_core::ModuleSpecifier;
use deno_core::RequestedModuleType;

use crate::ops::SharedState;

pub struct ObscuraModuleLoader {
    pub base_url: String,
    state: SharedState,
}

impl ObscuraModuleLoader {
    pub fn new(base_url: &str, state: SharedState) -> Self {
        ObscuraModuleLoader {
            base_url: base_url.to_string(),
            state,
        }
    }
}

fn io_err(msg: String) -> ModuleLoaderError {
    std::io::Error::new(std::io::ErrorKind::Other, msg).into()
}

impl ModuleLoader for ObscuraModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: deno_core::ResolutionKind,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        let base = if referrer.is_empty()
            || referrer.starts_with('<')
            || referrer == "."
            || referrer == "about:blank"
        {
            &self.base_url
        } else {
            referrer
        };

        deno_core::resolve_import(specifier, base).map_err(|e| e.into())
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleSpecifier>,
        _is_dyn_import: bool,
        _requested_module_type: RequestedModuleType,
    ) -> ModuleLoadResponse {
        let url = module_specifier.to_string();
        let state = self.state.clone();

        ModuleLoadResponse::Async(Pin::from(Box::new(async move {
            tracing::debug!("Loading ES module: {}", url);

            let http_client = {
                let gs = state.borrow();
                gs.http_client.clone()
            };

            let code = if let Some(client) = http_client {
                let parsed_url = url::Url::parse(&url)
                    .map_err(|e| io_err(format!("Invalid module URL {}: {}", url, e)))?;
                let resp = client
                    .fetch(&parsed_url)
                    .await
                    .map_err(|e| io_err(format!("Failed to fetch module {}: {}", url, e)))?;

                if !(200..300).contains(&resp.status) {
                    return Err(io_err(format!(
                        "Module {} returned HTTP {}",
                        url,
                        resp.status
                    )));
                }

                String::from_utf8_lossy(&resp.body).to_string()
            } else {
                let client = reqwest::Client::builder()
                    .build()
                    .map_err(|e| io_err(format!("HTTP client error: {}", e)))?;

                let resp = client
                    .get(&url)
                    .header("Accept", "application/javascript, text/javascript, */*")
                    .send()
                    .await
                    .map_err(|e| io_err(format!("Failed to fetch module {}: {}", url, e)))?;

                if !resp.status().is_success() {
                    return Err(io_err(format!(
                        "Module {} returned HTTP {}",
                        url,
                        resp.status()
                    )));
                }

                resp.text().await.map_err(|e| {
                    io_err(format!("Failed to read module body {}: {}", url, e))
                })?
            };

            let specifier = ModuleSpecifier::parse(&url)
                .map_err(|e| io_err(format!("Invalid module URL {}: {}", url, e)))?;

            Ok(ModuleSource::new(
                deno_core::ModuleType::JavaScript,
                ModuleSourceCode::String(code.into()),
                &specifier,
                None,
            ))
        })))
    }
}
