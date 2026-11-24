//! Service configuration (contract §1): where the services are, or that
//! they are switched off.
//!
//! `<ferro folder>/services.json`:
//!
//! ```json
//! { "base_url": "https://services.ferro.example", "enabled": true }
//! ```
//!
//! - Missing file: [`OFFICIAL_BASE_URL`].
//! - `"enabled": false` or an empty `base_url`: offline. Every call fails
//!   with `ferro.service.offline` before any socket is opened.
//! - [`ENV_URL`] (`FERRO_SERVICES_URL`) overrides `base_url` (tests,
//!   self-hosting); it never switches services back on when
//!   `"enabled": false`.
//! - A file that is not valid JSON, or a URL refused by [`check_url`],
//!   gives offline mode with [`ServiceConfig::problem`] set: a broken
//!   configuration never sends anything anywhere.

use crate::ServiceError;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// File name of the configuration in the Ferro folder.
pub const SERVICES_FILE: &str = "services.json";
/// Environment variable overriding `base_url`.
pub const ENV_URL: &str = "FERRO_SERVICES_URL";
/// The official endpoint, used when there is no `services.json`. The owner
/// sets the real address here before the first release that ships
/// services; until then the `.example` domain never resolves, so a default
/// build reaches nobody.
pub const OFFICIAL_BASE_URL: &str = "https://services.ferro.example";
/// Largest `services.json` read (bytes).
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// Where the services are, or that they are off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceConfig {
    base_url: Option<String>,
    home: Option<PathBuf>,
    problem: Option<String>,
}

/// Check a URL the client would call or open: `https://<host>...`, or
/// `http://` only to `localhost`, `127.0.0.1` or `[::1]` (TLS always for
/// any other host). No spaces, control characters or `user@` part.
/// Fails with `ferro.service.protocol`.
pub fn check_url(url: &str) -> Result<(), ServiceError> {
    let refuse = |why: &str| Err(ServiceError::protocol(format!("refused URL: {why}")));
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return refuse("it contains spaces or control characters");
    }
    let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return refuse("only https:// is allowed");
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return refuse("a user name in the URL is not allowed");
    }
    let host = host_of(authority);
    if host.is_empty() {
        return refuse("no host");
    }
    if !secure && !is_loopback_host(host) {
        return refuse("http:// is allowed only for localhost; use https://");
    }
    Ok(())
}

fn host_of(authority: &str) -> &str {
    if let Some(r) = authority.strip_prefix('[') {
        // [::1]:8080
        return r.split(']').next().map(|h| h.trim()).unwrap_or("");
    }
    authority.split(':').next().unwrap_or("")
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

/// True when `url` points at the local machine (`localhost`, `127.0.0.1`,
/// `[::1]`), whatever the scheme.
pub fn is_loopback_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or("");
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.contains('@') && is_loopback_host(host_of(authority))
}

impl ServiceConfig {
    /// The configuration of this machine: `services.json` in the Ferro
    /// folder (`FERRO_HOME`), overridden by `FERRO_SERVICES_URL`. Never
    /// fails; problems give offline mode (see the module doc).
    pub fn load() -> ServiceConfig {
        let env = std::env::var(ENV_URL).ok();
        ServiceConfig::load_from(crate::ferro_dir().as_deref(), env.as_deref())
    }

    /// [`Self::load`] with the Ferro folder and the `FERRO_SERVICES_URL`
    /// value given (tests). `home: None`: no Ferro folder (no file, nowhere
    /// to keep tokens).
    pub fn load_from(home: Option<&Path>, env_url: Option<&str>) -> ServiceConfig {
        let mut base = Some(OFFICIAL_BASE_URL.to_string());
        let mut enabled = true;
        let mut problem = None;
        if let Some(path) = home.map(|h| h.join(SERVICES_FILE)) {
            match read_file(&path) {
                Ok(None) => {}
                Ok(Some(v)) => {
                    if v.get("enabled").and_then(Value::as_bool) == Some(false) {
                        enabled = false;
                    }
                    match v.get("base_url") {
                        None | Some(Value::Null) => {}
                        Some(Value::String(s)) if s.trim().is_empty() => enabled = false,
                        Some(Value::String(s)) => base = Some(s.trim().to_string()),
                        Some(_) => {
                            problem = Some(format!("{SERVICES_FILE}: `base_url` must be text"));
                        }
                    }
                }
                Err(m) => problem = Some(m),
            }
        }
        if let Some(e) = env_url.map(str::trim).filter(|e| !e.is_empty()) {
            base = Some(e.to_string());
        }
        if let Some(b) = &base {
            if let Err(e) = check_url(b) {
                problem.get_or_insert(format!("{}: {}", b, e.message));
            }
        }
        let base = if enabled && problem.is_none() {
            base.map(|b| b.trim_end_matches('/').to_string())
        } else {
            None
        };
        ServiceConfig {
            base_url: base,
            home: home.map(Path::to_path_buf),
            problem,
        }
    }

    /// Offline, with `home` as the Ferro folder (or none).
    pub fn offline(home: Option<&Path>) -> ServiceConfig {
        ServiceConfig {
            base_url: None,
            home: home.map(Path::to_path_buf),
            problem: None,
        }
    }

    /// Services at `base_url`, with `home` as the Ferro folder. Fails with
    /// `ferro.service.protocol` when [`check_url`] refuses the URL.
    pub fn online(base_url: &str, home: Option<&Path>) -> Result<ServiceConfig, ServiceError> {
        check_url(base_url)?;
        Ok(ServiceConfig {
            base_url: Some(base_url.trim_end_matches('/').to_string()),
            home: home.map(Path::to_path_buf),
            problem: None,
        })
    }

    /// The base URL without a trailing `/`; `None` when offline.
    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    /// True when services are off: no call touches the network.
    pub fn is_offline(&self) -> bool {
        self.base_url.is_none()
    }

    /// True when the services run on this machine (a mock or a self-hosted
    /// server on `localhost`).
    pub fn is_loopback(&self) -> bool {
        self.base_url.as_deref().is_some_and(is_loopback_url)
    }

    /// The Ferro folder, where `account.json` and `licenses.json` live.
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    /// Why the configuration fell back to offline mode (a broken
    /// `services.json` or a refused URL), for the user.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// The full URL of an endpoint path (`/v1/...`). Fails with
    /// `ferro.service.offline` when offline.
    pub fn url(&self, path: &str) -> Result<String, ServiceError> {
        let base = self.base_url.as_deref().ok_or_else(ServiceError::offline)?;
        Ok(format!("{base}{path}"))
    }
}

fn read_file(path: &Path) -> Result<Option<Value>, String> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {SERVICES_FILE}: {e}")),
    };
    let mut text = String::new();
    file.take(MAX_FILE_BYTES)
        .read_to_string(&mut text)
        .map_err(|e| format!("cannot read {SERVICES_FILE}: {e}"))?;
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("{SERVICES_FILE} is not JSON: {e}"))?;
    if !v.is_object() {
        return Err(format!("{SERVICES_FILE} must be a JSON object"));
    }
    Ok(Some(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ferro-services-config-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn urls_need_tls_except_for_localhost() {
        for ok in [
            "https://services.ferro.example",
            "https://a.b:8443/x?y#z",
            "http://localhost:8080",
            "http://127.0.0.1:1/v1",
            "http://[::1]:9000",
            "http://LOCALHOST",
        ] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://services.ferro.example",
            "http://127.0.0.2",
            "http://localhost.evil.com",
            "http://localhost@evil.com",
            "https://user@host",
            "ftp://x",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://",
            "https://a b",
            "",
        ] {
            let e = check_url(bad).unwrap_err();
            assert_eq!(e.code, crate::codes::PROTOCOL, "{bad}");
        }
        assert!(is_loopback_url("http://127.0.0.1:5/x"));
        assert!(!is_loopback_url("https://localhost.evil.com"));
    }

    #[test]
    fn missing_file_is_the_official_endpoint_and_env_overrides() {
        let home = temp("default");
        let c = ServiceConfig::load_from(Some(&home), None);
        assert_eq!(c.base_url(), Some(OFFICIAL_BASE_URL));
        assert!(!c.is_offline() && !c.is_loopback());
        let c = ServiceConfig::load_from(Some(&home), Some("http://127.0.0.1:9/"));
        assert_eq!(c.base_url(), Some("http://127.0.0.1:9"));
        assert!(c.is_loopback());
        assert_eq!(
            c.url("/v1/licenses").unwrap(),
            "http://127.0.0.1:9/v1/licenses"
        );
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn disabled_empty_broken_or_plain_http_files_are_offline() {
        let home = temp("offline");
        let file = home.join(SERVICES_FILE);
        for (text, problem) in [
            (
                r#"{"base_url": "https://x.example", "enabled": false}"#,
                false,
            ),
            (r#"{"base_url": ""}"#, false),
            ("not json", true),
            (r#"{"base_url": 3}"#, true),
            (r#"{"base_url": "http://x.example"}"#, true),
        ] {
            std::fs::write(&file, text).unwrap();
            // The env URL never switches a disabled configuration back on.
            let c = ServiceConfig::load_from(Some(&home), Some("http://127.0.0.1:9"));
            let c = if problem {
                ServiceConfig::load_from(Some(&home), None)
            } else {
                c
            };
            assert!(c.is_offline(), "{text}");
            assert_eq!(c.problem().is_some(), problem, "{text}");
            assert_eq!(c.url("/v1/x").unwrap_err().code, crate::codes::OFFLINE);
        }
        std::fs::write(
            &file,
            r#"{"base_url": "https://self.example/", "enabled": true}"#,
        )
        .unwrap();
        let c = ServiceConfig::load_from(Some(&home), None);
        assert_eq!(c.base_url(), Some("https://self.example"));
        assert_eq!(c.home(), Some(home.as_path()));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn online_refuses_plain_http_to_other_hosts() {
        assert!(ServiceConfig::online("http://evil.example", None).is_err());
        assert!(ServiceConfig::online("http://localhost:1", None).is_ok());
        assert!(ServiceConfig::offline(None).is_offline());
    }
}
