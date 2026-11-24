//! A small blocking HTTP client for the services contract: JSON bodies,
//! timeouts, `User-Agent: Ferro/<version>`, `Authorization: Bearer` when a
//! token is given, and the `ferro.service.*` failure codes.
//!
//! Built on `ureq` with rustls (no OpenSSL). Blocking: call it from a
//! worker thread, never from the UI thread. Offline, a client holds no
//! HTTP agent at all, so no call can reach the network.
//!
//! Status codes:
//! - 2xx and 4xx (except 401): a [`Reply`] (the contract carries errors
//!   such as `authorization_pending` in 4xx JSON bodies);
//! - 401: `ferro.service.unauthorized`;
//! - 3xx (redirects are not followed for API calls): `ferro.service.protocol`;
//! - 5xx, connection, DNS and timeout failures: `ferro.service.unavailable`.

use crate::account::Secret;
use crate::config::{check_url, is_loopback_url, ServiceConfig};
use crate::text::sanitize;
use crate::{codes, ServiceError};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

/// Time limit of one API call (connect + answer).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(20);
/// Time limit of one download.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// Largest JSON answer read (bytes).
pub const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;

/// An answer of the server with a 2xx or 4xx status (401 excepted).
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    /// HTTP status.
    pub status: u16,
    /// JSON body (`null` when empty).
    pub body: Value,
}

impl Reply {
    /// True for a 2xx status.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The `error` string of an error body (`authorization_pending`, ...).
    pub fn error(&self) -> Option<&str> {
        self.body.get("error").and_then(Value::as_str)
    }

    /// The body of a 2xx answer; a 4xx fails with `ferro.service.protocol`
    /// naming the status and the server's (sanitised) `error` / `message`.
    pub fn into_success(self) -> Result<Value, ServiceError> {
        if self.is_success() {
            return Ok(self.body);
        }
        let why = self
            .body
            .get("message")
            .or_else(|| self.body.get("error"))
            .and_then(Value::as_str)
            .map(|m| sanitize(m, 200))
            .unwrap_or_default();
        Err(ServiceError::protocol(format!(
            "the server refused the request ({}){}{}",
            self.status,
            if why.is_empty() { "" } else { ": " },
            why
        )))
    }
}

/// The blocking client of one [`ServiceConfig`].
#[derive(Debug, Clone)]
pub struct ServiceClient {
    config: ServiceConfig,
    agent: Option<ureq::Agent>,
}

fn agent(timeout: Duration, loopback: bool, follow_redirects: bool) -> ureq::Agent {
    let mut b = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(timeout))
        .timeout_connect(Some(Duration::from_secs(10)))
        .user_agent(format!("Ferro/{}", env!("CARGO_PKG_VERSION")))
        .max_redirects(if follow_redirects { 5 } else { 0 })
        .max_redirects_will_error(false);
    if loopback {
        // Local servers are reached directly, never through a proxy.
        b = b.proxy(None);
    } else {
        // Redirects of downloads may go to a CDN, but never to plain http.
        b = b.https_only(true);
    }
    b.build().new_agent()
}

fn transport_error(e: ureq::Error) -> ServiceError {
    let why = match &e {
        ureq::Error::Timeout(_) => "the server did not answer in time".to_string(),
        ureq::Error::HostNotFound => "the server's name could not be resolved".to_string(),
        ureq::Error::Io(io) => format!("network error: {}", io.kind()),
        ureq::Error::ConnectionFailed => "cannot connect to the server".to_string(),
        ureq::Error::BodyExceedsLimit(n) => {
            return ServiceError::protocol(format!("the answer is larger than {n} bytes"))
        }
        ureq::Error::Json(_) => return ServiceError::protocol("the answer is not JSON"),
        other => format!("network error: {}", sanitize(&other.to_string(), 200)),
    };
    ServiceError::new(codes::UNAVAILABLE, why)
}

fn status_error(status: u16) -> Option<ServiceError> {
    match status {
        401 => Some(ServiceError::new(
            codes::UNAUTHORIZED,
            "not signed in, or the sign-in has expired",
        )),
        500..=599 => Some(ServiceError::new(
            codes::UNAVAILABLE,
            format!("the server failed ({status})"),
        )),
        300..=399 => Some(ServiceError::protocol(format!(
            "unexpected redirect ({status})"
        ))),
        200..=299 | 400..=499 => None,
        _ => Some(ServiceError::protocol(format!(
            "unexpected status {status}"
        ))),
    }
}

impl ServiceClient {
    /// A client of `config`. Offline, it holds no HTTP agent.
    pub fn new(config: ServiceConfig) -> ServiceClient {
        let agent =
            (!config.is_offline()).then(|| agent(CALL_TIMEOUT, config.is_loopback(), false));
        ServiceClient { config, agent }
    }

    /// Its configuration.
    pub fn config(&self) -> &ServiceConfig {
        &self.config
    }

    /// True when services are off.
    pub fn is_offline(&self) -> bool {
        self.agent.is_none()
    }

    fn reply(
        &self,
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<Reply, ServiceError> {
        let mut res = result.map_err(transport_error)?;
        let status = res.status().as_u16();
        if let Some(e) = status_error(status) {
            return Err(e);
        }
        let bytes = res
            .body_mut()
            .with_config()
            .limit(MAX_JSON_BYTES)
            .read_to_vec()
            .map_err(transport_error)?;
        let body = if bytes.iter().all(u8::is_ascii_whitespace) {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .map_err(|_| ServiceError::protocol("the answer is not JSON"))?
        };
        Ok(Reply { status, body })
    }

    /// `GET <base><path>` (path starts with `/v1/`). Fails with
    /// `ferro.service.offline` when offline, or as the module doc says.
    pub fn get(&self, path: &str, token: Option<&Secret>) -> Result<Reply, ServiceError> {
        let (agent, url) = self.target(path)?;
        let mut req = agent.get(&url).header("Accept", "application/json");
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {}", t.expose()));
        }
        self.reply(req.call())
    }

    /// `POST <base><path>` with a JSON body. Fails like [`Self::get`].
    pub fn post(
        &self,
        path: &str,
        body: &Value,
        token: Option<&Secret>,
    ) -> Result<Reply, ServiceError> {
        let (agent, url) = self.target(path)?;
        let mut req = agent.post(&url).header("Accept", "application/json");
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {}", t.expose()));
        }
        self.reply(req.send_json(body))
    }

    fn target(&self, path: &str) -> Result<(&ureq::Agent, String), ServiceError> {
        let agent = self.agent.as_ref().ok_or_else(ServiceError::offline)?;
        Ok((agent, self.config.url(path)?))
    }

    /// Download `url` (absolute; [`check_url`] applies, and redirects never
    /// go to plain http) into the file `dest`, at most `max_bytes`, and
    /// return its size and SHA-256. The token is never sent to a download
    /// URL. Fails with `ferro.service.offline` when offline,
    /// `ferro.service.protocol` for a refused URL, a non-2xx status or a
    /// file larger than `max_bytes`, `ferro.service.unavailable` on network
    /// failures; a partial file is removed.
    pub fn download(
        &self,
        url: &str,
        max_bytes: u64,
        dest: &Path,
    ) -> Result<(u64, [u8; 32]), ServiceError> {
        if self.agent.is_none() {
            return Err(ServiceError::offline());
        }
        check_url(url)?;
        let agent = agent(
            DOWNLOAD_TIMEOUT,
            is_loopback_url(url),
            !is_loopback_url(url),
        );
        let res = agent.get(url).call().map_err(transport_error)?;
        let status = res.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(status_error(status).unwrap_or_else(|| {
                ServiceError::protocol(format!("the download failed ({status})"))
            }));
        }
        let r = copy_hashed(res.into_body().into_reader(), dest, max_bytes);
        if r.is_err() {
            let _ = std::fs::remove_file(dest);
        }
        r
    }
}

fn copy_hashed(
    mut reader: impl Read,
    dest: &Path,
    max_bytes: u64,
) -> Result<(u64, [u8; 32]), ServiceError> {
    let mut file = std::fs::File::create(dest)
        .map_err(|e| ServiceError::protocol(format!("cannot write the download: {e}")))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(ServiceError::new(
                    codes::UNAVAILABLE,
                    format!("the download broke off: {}", e.kind()),
                ))
            }
        };
        total += n as u64;
        if total > max_bytes {
            return Err(ServiceError::protocol(format!(
                "the download is larger than {max_bytes} bytes"
            )));
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| ServiceError::protocol(format!("cannot write the download: {e}")))?;
    }
    file.flush()
        .map_err(|e| ServiceError::protocol(format!("cannot write the download: {e}")))?;
    Ok((total, hasher.finalize().into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_client_has_no_agent_and_fails_offline() {
        let c = ServiceClient::new(ServiceConfig::offline(None));
        assert!(c.is_offline());
        assert_eq!(
            c.get("/v1/licenses", None).unwrap_err().code,
            codes::OFFLINE
        );
        let e = c.post("/v1/auth/device", &Value::Null, None).unwrap_err();
        assert_eq!(e.code, codes::OFFLINE);
        let dest = std::env::temp_dir().join("ferro-never-written");
        let e = c
            .download("https://x.example/a.zip", 10, &dest)
            .unwrap_err();
        assert_eq!(e.code, codes::OFFLINE);
        assert!(!dest.exists());
    }

    #[test]
    fn statuses_map_to_codes() {
        assert_eq!(status_error(401).unwrap().code, codes::UNAUTHORIZED);
        assert_eq!(status_error(503).unwrap().code, codes::UNAVAILABLE);
        assert_eq!(status_error(302).unwrap().code, codes::PROTOCOL);
        assert!(status_error(200).is_none() && status_error(400).is_none());
        let r = Reply {
            status: 404,
            body: serde_json::json!({"error": "no\u{202E} such plugin"}),
        };
        let e = r.into_success().unwrap_err();
        assert_eq!(e.code, codes::PROTOCOL);
        assert!(
            e.message.ends_with("(404): no such plugin"),
            "{}",
            e.message
        );
    }

    #[test]
    fn copy_stops_at_the_limit() {
        let dest = std::env::temp_dir().join(format!("ferro-dl-{}", uuid::Uuid::new_v4().simple()));
        let (n, sha) = copy_hashed(&b"abc"[..], &dest, 3).unwrap();
        assert_eq!(n, 3);
        assert_eq!(
            crate::text::hex(&sha),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            copy_hashed(&b"abcd"[..], &dest, 3).unwrap_err().code,
            codes::PROTOCOL
        );
        let _ = std::fs::remove_file(dest);
    }
}
