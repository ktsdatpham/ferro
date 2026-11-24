//! Account sign-in (contract §2): the OAuth 2.0 device authorization grant
//! (RFC 8628). The app never shows a password field: it shows a short user
//! code, opens the verification page in the browser and polls until the
//! user has signed in there.
//!
//! Token storage: `<ferro folder>/account.json`, written with 0600
//! permissions on unix (owner only; on Windows the file inherits the
//! user-profile ACL). The OS keychain (`keyring` crate) was not used: it
//! needs D-Bus development headers on Linux CI and could not be checked
//! on the macOS / Windows runners from here; it can replace
//! [`save`] / [`load`] / [`clear`] later without changing callers.
//!
//! Tokens are held in [`Secret`], whose `Debug` never prints them and which
//! has no `Display`: they cannot reach a log, a problem report, an MCP
//! answer or a plugin's environment by formatting.

use crate::http::{Reply, ServiceClient};
use crate::text::{now_s, sanitize};
use crate::{codes, ServiceError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// File name of the stored sign-in in the Ferro folder.
pub const ACCOUNT_FILE: &str = "account.json";
/// Polling interval when the server gives none (RFC 8628 §3.2).
pub const DEFAULT_INTERVAL_S: u64 = 5;
/// Seconds added to the interval on `slow_down` (RFC 8628 §3.5).
pub const SLOW_DOWN_S: u64 = 5;
/// Longest polling interval accepted from the server.
pub const MAX_INTERVAL_S: u64 = 60;
/// Longest device-code lifetime accepted from the server (seconds).
pub const MAX_EXPIRES_IN_S: u64 = 3600;
/// An access token this close to its expiry is refreshed before use.
pub const REFRESH_MARGIN_S: i64 = 60;

/// A token or code. `Debug` prints `Secret(..)`, there is no `Display`;
/// [`Secret::expose`] is the one way to read it (HTTP headers and bodies,
/// `account.json`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// Wrap a token.
    pub fn new(s: impl Into<String>) -> Secret {
        Secret(s.into())
    }

    /// The token text, for an HTTP request or the token file only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// The signed-in account, as the server describes it (sanitised).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Account id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// E-mail address.
    pub email: String,
}

/// The tokens of a sign-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountTokens {
    /// Sent as `Authorization: Bearer` on signed-in calls.
    pub access_token: Secret,
    /// Exchanged for a new access token (`/v1/auth/refresh`).
    pub refresh_token: Secret,
    /// When the access token expires (Unix seconds).
    pub expires_at_s: i64,
}

/// A sign-in: who, and the tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedIn {
    /// The account.
    pub account: Account,
    /// Its tokens.
    pub tokens: AccountTokens,
}

/// The answer of `POST /v1/auth/device`: what to show the user and how
/// to poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStart {
    /// Sent back when polling (a credential: never shown).
    pub device_code: Secret,
    /// The short code the user types on the web page (sanitised).
    pub user_code: String,
    /// The page to open (checked by `check_url`).
    pub verification_uri: String,
    /// Seconds between polls (1 to [`MAX_INTERVAL_S`]).
    pub interval_s: u64,
    /// Seconds before the device code expires (at most
    /// [`MAX_EXPIRES_IN_S`]).
    pub expires_in_s: u64,
}

/// What one poll of `POST /v1/auth/token` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollStep {
    /// The user has not finished yet (`authorization_pending`).
    Pending,
    /// Poll more slowly (`slow_down`).
    SlowDown,
    /// Signed in.
    Done(SignedIn),
}

fn text(v: &Value, key: &str, max: usize) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(|s| sanitize(s, max))
        .filter(|s| !s.is_empty())
}

fn secret(v: &Value, key: &str) -> Option<Secret> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 8192)
        .map(Secret::new)
}

fn account_of(v: &Value) -> Result<Account, ServiceError> {
    let a = v
        .get("account")
        .ok_or_else(|| ServiceError::protocol("the answer has no `account`"))?;
    let id = text(a, "id", 128).ok_or_else(|| ServiceError::protocol("the account has no `id`"))?;
    Ok(Account {
        name: text(a, "name", 120).unwrap_or_else(|| id.clone()),
        email: text(a, "email", 254).unwrap_or_default(),
        id,
    })
}

fn tokens_of(v: &Value, old_refresh: Option<&Secret>) -> Result<AccountTokens, ServiceError> {
    let access_token =
        secret(v, "access_token").ok_or_else(|| ServiceError::protocol("no `access_token`"))?;
    let refresh_token = secret(v, "refresh_token")
        .or_else(|| old_refresh.cloned())
        .ok_or_else(|| ServiceError::protocol("no `refresh_token`"))?;
    let expires_in = v.get("expires_in").and_then(Value::as_i64).unwrap_or(3600);
    Ok(AccountTokens {
        access_token,
        refresh_token,
        expires_at_s: now_s().saturating_add(expires_in.clamp(0, 10 * 365 * 86_400)),
    })
}

/// Start a sign-in: `POST /v1/auth/device`. Fails with
/// `ferro.service.offline` / `unavailable`, or `ferro.service.protocol`
/// when the answer lacks a field or its `verification_uri` is refused
/// (not https, except to localhost).
pub fn start_device_flow(client: &ServiceClient) -> Result<DeviceStart, ServiceError> {
    let v = client
        .post("/v1/auth/device", &json!({}), None)?
        .into_success()?;
    let device_code =
        secret(&v, "device_code").ok_or_else(|| ServiceError::protocol("no `device_code`"))?;
    let user_code =
        text(&v, "user_code", 32).ok_or_else(|| ServiceError::protocol("no `user_code`"))?;
    let verification_uri = v
        .get("verification_uri")
        .and_then(Value::as_str)
        .ok_or_else(|| ServiceError::protocol("no `verification_uri`"))?
        .to_string();
    crate::config::check_url(&verification_uri)?;
    let interval_s = v
        .get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_INTERVAL_S)
        .clamp(1, MAX_INTERVAL_S);
    let expires_in_s = v
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(600)
        .min(MAX_EXPIRES_IN_S);
    Ok(DeviceStart {
        device_code,
        user_code,
        verification_uri,
        interval_s,
        expires_in_s,
    })
}

/// Poll once: `POST /v1/auth/token {device_code}`. Fails with
/// `ferro.service.protocol` when the user refused (`access_denied`), the
/// code expired (`expired_token`) or the answer is malformed.
pub fn poll_token(client: &ServiceClient, start: &DeviceStart) -> Result<PollStep, ServiceError> {
    let r: Reply = client.post(
        "/v1/auth/token",
        &json!({ "device_code": start.device_code.expose() }),
        None,
    )?;
    match r.error() {
        Some("authorization_pending") => return Ok(PollStep::Pending),
        Some("slow_down") => return Ok(PollStep::SlowDown),
        Some("access_denied") => return Err(ServiceError::protocol("the sign-in was refused")),
        Some("expired_token") => {
            return Err(ServiceError::protocol(
                "the sign-in code expired: start again",
            ))
        }
        _ => {}
    }
    let v = r.into_success()?;
    Ok(PollStep::Done(SignedIn {
        account: account_of(&v)?,
        tokens: tokens_of(&v, None)?,
    }))
}

/// Poll until the user has signed in, honouring `interval` and
/// `slow_down`, and stopping at `expires_in` (`ferro.service.protocol`).
/// `sleep` waits between polls (the app passes `std::thread::sleep`; tests
/// pass a no-op). Returns `Ok(None)` when `cancel` is set. Blocking: run it
/// on a worker thread.
pub fn wait_for_sign_in(
    client: &ServiceClient,
    start: &DeviceStart,
    cancel: &AtomicBool,
    sleep: &mut dyn FnMut(Duration),
) -> Result<Option<SignedIn>, ServiceError> {
    let begun = Instant::now();
    let mut interval = start.interval_s;
    let mut waited = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if waited >= start.expires_in_s || begun.elapsed().as_secs() >= start.expires_in_s {
            return Err(ServiceError::protocol(
                "the sign-in code expired: start again",
            ));
        }
        sleep(Duration::from_secs(interval));
        waited += interval;
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        match poll_token(client, start)? {
            PollStep::Pending => {}
            PollStep::SlowDown => interval = (interval + SLOW_DOWN_S).min(MAX_INTERVAL_S),
            PollStep::Done(s) => return Ok(Some(s)),
        }
    }
}

/// New tokens: `POST /v1/auth/refresh {refresh_token}`. The account is
/// kept unless the answer carries one. Fails with
/// `ferro.service.unauthorized` when the refresh token is no longer valid
/// (401, or a 4xx answer).
pub fn refresh(client: &ServiceClient, signed: &SignedIn) -> Result<SignedIn, ServiceError> {
    let r = client.post(
        "/v1/auth/refresh",
        &json!({ "refresh_token": signed.tokens.refresh_token.expose() }),
        None,
    )?;
    if !r.is_success() {
        return Err(ServiceError::new(
            codes::UNAUTHORIZED,
            "the sign-in has expired: sign in again",
        ));
    }
    let v = r.body;
    Ok(SignedIn {
        account: account_of(&v).unwrap_or_else(|_| signed.account.clone()),
        tokens: tokens_of(&v, Some(&signed.tokens.refresh_token))?,
    })
}

/// Sign out on the server: `POST /v1/auth/revoke {refresh_token}` with the
/// access token. Callers [`clear`] the stored tokens whatever this returns.
pub fn revoke(client: &ServiceClient, signed: &SignedIn) -> Result<(), ServiceError> {
    client
        .post(
            "/v1/auth/revoke",
            &json!({ "refresh_token": signed.tokens.refresh_token.expose() }),
            Some(&signed.tokens.access_token),
        )?
        .into_success()
        .map(|_| ())
}

/// Run a signed-in call with a fresh access token: refresh first when the
/// token is about to expire, and once more (then retry) when the call
/// fails with `ferro.service.unauthorized`. `signed` is updated in place;
/// `changed` tells the caller to [`save`] it. Fails with
/// `ferro.service.unauthorized` when the refresh fails too.
pub fn call_signed_in<T>(
    client: &ServiceClient,
    signed: &mut SignedIn,
    changed: &mut bool,
    f: impl Fn(&Secret) -> Result<T, ServiceError>,
) -> Result<T, ServiceError> {
    if signed.tokens.expires_at_s <= now_s() + REFRESH_MARGIN_S {
        *signed = refresh(client, signed)?;
        *changed = true;
    }
    match f(&signed.tokens.access_token) {
        Err(e) if e.code == codes::UNAUTHORIZED => {
            *signed = refresh(client, signed)?;
            *changed = true;
            f(&signed.tokens.access_token)
        }
        r => r,
    }
}

/// Write `signed` to `<home>/account.json` (0600 on unix, written to a
/// temporary file first, then renamed). Fails with a message when the
/// folder cannot be written.
pub fn save(home: &Path, signed: &SignedIn) -> Result<(), String> {
    let text = serde_json::to_string_pretty(signed).map_err(|e| e.to_string())?;
    write_private(&home.join(ACCOUNT_FILE), text.as_bytes())
}

/// Write `bytes` to `path` readable by the owner only (0600 on unix),
/// through a temporary file in the same folder.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let dir = path.parent().ok_or("no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let r = opts
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes).and_then(|_| f.sync_all()))
        .and_then(|_| std::fs::rename(&tmp, path));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r.map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// The stored sign-in, if any (a broken file counts as signed out).
pub fn load(home: &Path) -> Option<SignedIn> {
    let text = std::fs::read_to_string(home.join(ACCOUNT_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Forget the stored sign-in (remove `account.json`).
pub fn clear(home: &Path) {
    let _ = std::fs::remove_file(home.join(ACCOUNT_FILE));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed(token: &str) -> SignedIn {
        SignedIn {
            account: Account {
                id: "a1".into(),
                name: "An".into(),
                email: "an@example.com".into(),
            },
            tokens: AccountTokens {
                access_token: Secret::new(token),
                refresh_token: Secret::new(format!("{token}-refresh")),
                expires_at_s: 0,
            },
        }
    }

    #[test]
    fn secrets_never_print() {
        let s = signed("tok-SECRET-123");
        let printed = format!("{s:?} {:?} {:#?}", s.tokens, s);
        assert!(!printed.contains("SECRET"), "{printed}");
        assert!(printed.contains("Secret(..)"));
        assert_eq!(s.tokens.access_token.expose(), "tok-SECRET-123");
    }

    #[test]
    fn account_file_round_trips_owner_only() {
        let home =
            std::env::temp_dir().join(format!("ferro-acct-{}", uuid::Uuid::new_v4().simple()));
        assert!(load(&home).is_none());
        let s = signed("t1");
        save(&home, &s).unwrap();
        assert_eq!(load(&home), Some(s));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(home.join(ACCOUNT_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(home.join(ACCOUNT_FILE), "broken").unwrap();
        assert!(load(&home).is_none());
        clear(&home);
        assert!(!home.join(ACCOUNT_FILE).exists());
        // Only the account file was there: no temporary files left.
        assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn token_answers_are_checked_and_sanitised() {
        let v = json!({"access_token": "a", "refresh_token": "r", "expires_in": 10,
                       "account": {"id": "u1", "name": "Tên\u{202E} dài", "email": "x@y"}});
        assert_eq!(account_of(&v).unwrap().name, "Tên dài");
        assert!(tokens_of(&v, None).unwrap().expires_at_s > now_s());
        assert!(account_of(&json!({"account": {"name": "x"}})).is_err());
        assert!(tokens_of(&json!({"access_token": "a"}), None).is_err());
        let old = Secret::new("old");
        assert_eq!(
            tokens_of(&json!({"access_token": "a"}), Some(&old))
                .unwrap()
                .refresh_token,
            old
        );
    }
}
