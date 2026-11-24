//! Licence tokens and entitlements (contract §3).
//!
//! A [`LicenseToken`] is `base64url(payload) "." base64url(signature)`: an
//! Ed25519 signature over the payload bytes (the decoded JSON), checked
//! offline against a public key chosen by the payload's `kid` in a
//! [`KeyRing`]. A token past `exp` keeps working for [`GRACE_S`] (14 days)
//! with a warning, then fails with `ferro.license.expired`.
//! [`Entitlements`] is the union of the valid tokens' `ent` lists.
//!
//! Keys. [`PRODUCTION_KEYS`] holds the official public keys and ships
//! empty: **the owner adds the real public key there** (never a private
//! key: those live only on the licence server). The one other entry of the
//! built-in list is the clearly marked DEVELOPMENT key
//! ([`DEVELOPMENT_KID`]): its public half is read from
//! `FERRO_DEV_LICENSE_KEY` (printed by `ferro-services-mock`) and it is
//! trusted **only while the services URL is on this machine**, so a token
//! signed by a development key never unlocks anything against the
//! official server. No private key is in this repository; the mock and the
//! tests create theirs at run time.
//!
//! The token cache is `<ferro folder>/licenses.json` (`{"tokens": [..]}`):
//! tokens are signed, so plain storage is fine.

use crate::account::Secret;
use crate::config::ServiceConfig;
use crate::http::ServiceClient;
use crate::text::{b64url_decode, format_rfc3339, parse_rfc3339, sanitize};
use crate::{codes, ServiceError};
use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;

/// Grace period after `exp` (seconds): 14 days.
pub const GRACE_S: i64 = 14 * 86_400;
/// File name of the token cache in the Ferro folder.
pub const LICENSES_FILE: &str = "licenses.json";
/// Environment variable with the DEVELOPMENT public key (base64url, 32
/// bytes), honoured only for services on `localhost`.
pub const DEV_KEY_ENV: &str = "FERRO_DEV_LICENSE_KEY";
/// `kid` of the DEVELOPMENT key.
pub const DEVELOPMENT_KID: &str = "dev";
/// Largest token accepted (bytes).
pub const MAX_TOKEN_BYTES: usize = 16 * 1024;
/// Most tokens kept in the cache.
pub const MAX_TOKENS: usize = 256;

/// The official public keys (`kid`, Ed25519 public key bytes). Empty in
/// the open repository until the owner adds the licence server's public
/// key here, e.g. `("2026-1", [0x12, 0x34, ...])`. Rotating keys = adding
/// an entry; old entries stay while their tokens live.
pub const PRODUCTION_KEYS: &[(&str, [u8; 32])] = &[];

/// Public keys by `kid`.
#[derive(Debug, Clone, Default)]
pub struct KeyRing {
    keys: Vec<(String, VerifyingKey, bool)>,
}

impl KeyRing {
    /// No keys: every token is refused.
    pub fn empty() -> KeyRing {
        KeyRing::default()
    }

    /// The built-in list for `config`: [`PRODUCTION_KEYS`], plus the
    /// DEVELOPMENT key from `FERRO_DEV_LICENSE_KEY` when the services run
    /// on this machine ([`ServiceConfig::is_loopback`]).
    pub fn builtin(config: &ServiceConfig) -> KeyRing {
        let env = std::env::var(DEV_KEY_ENV).ok();
        KeyRing::builtin_with(config, env.as_deref())
    }

    /// [`Self::builtin`] with the `FERRO_DEV_LICENSE_KEY` value given.
    pub fn builtin_with(config: &ServiceConfig, dev_key: Option<&str>) -> KeyRing {
        let mut ring = KeyRing::empty();
        for (kid, bytes) in PRODUCTION_KEYS {
            ring = ring.with_key(kid, bytes, false);
        }
        if config.is_loopback() {
            let bytes = dev_key
                .and_then(|k| b64url_decode(k.trim()))
                .and_then(|b| <[u8; 32]>::try_from(b).ok());
            if let Some(b) = bytes {
                ring = ring.with_key(DEVELOPMENT_KID, &b, true);
            }
        }
        ring
    }

    /// Add a key (ignored when the bytes are not a valid Ed25519 public
    /// key). `development` marks keys that are not official.
    pub fn with_key(mut self, kid: &str, public_key: &[u8; 32], development: bool) -> KeyRing {
        if let Ok(k) = VerifyingKey::from_bytes(public_key) {
            self.keys.retain(|(id, _, _)| id != kid);
            self.keys.push((kid.to_string(), k, development));
        }
        self
    }

    /// The key with this `kid`.
    pub fn key(&self, kid: &str) -> Option<&VerifyingKey> {
        self.keys
            .iter()
            .find(|(id, _, _)| id == kid)
            .map(|(_, k, _)| k)
    }

    /// True when the ring holds no key.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// True when the key with this `kid` is a development key.
    pub fn is_development(&self, kid: &str) -> bool {
        self.keys.iter().any(|(id, _, d)| id == kid && *d)
    }
}

/// A verified licence token's payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicensePayload {
    /// Format version (1).
    pub v: u64,
    /// Account id.
    pub sub: String,
    /// Issued at (Unix seconds).
    pub iat_s: i64,
    /// Expires at (Unix seconds); the grace period follows.
    pub exp_s: i64,
    /// Entitlement ids (`plugin:<id>`, `cloud.sync`, ...).
    pub ent: Vec<String>,
    /// Device limit (enforced by the server, informative here).
    pub dev: Option<u64>,
    /// Key id.
    pub kid: String,
}

/// Where a valid token stands in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    /// Before `exp`.
    Valid,
    /// Past `exp`, inside the grace period: works, with a warning.
    Grace {
        /// Whole days left before it stops working (0 on the last day).
        days_left: i64,
    },
}

/// A licence token that passed verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicenseToken {
    /// The token text as issued.
    pub raw: String,
    /// Its payload.
    pub payload: LicensePayload,
    /// Valid or in grace at the time it was checked.
    pub state: TokenState,
}

fn invalid(why: &str) -> ServiceError {
    ServiceError::new(
        codes::LICENSE_INVALID,
        format!("licence token refused: {why}"),
    )
}

fn is_entitlement_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | ':' | '_' | '-')
        })
}

impl LicenseToken {
    /// Verify `raw` against `keys` at `now_s` (Unix seconds): two
    /// base64url parts, a JSON payload with `v` = 1, a known `kid`, a valid
    /// Ed25519 signature over the payload bytes, RFC 3339 `iat` / `exp`.
    /// Fails with `ferro.license.invalid`, or `ferro.license.expired` when
    /// `now_s` is past `exp` + [`GRACE_S`]. Entitlement ids that are not
    /// `[a-z0-9.:_-]` are dropped.
    pub fn verify(raw: &str, keys: &KeyRing, now_s: i64) -> Result<LicenseToken, ServiceError> {
        let raw = raw.trim();
        if raw.len() > MAX_TOKEN_BYTES {
            return Err(invalid("too long"));
        }
        let (p64, s64) = raw
            .split_once('.')
            .ok_or_else(|| invalid("not two parts"))?;
        let payload_bytes =
            b64url_decode(p64).ok_or_else(|| invalid("payload is not base64url"))?;
        let sig_bytes = b64url_decode(s64).ok_or_else(|| invalid("signature is not base64url"))?;
        let v: Value =
            serde_json::from_slice(&payload_bytes).map_err(|_| invalid("payload is not JSON"))?;
        if v.get("v").and_then(Value::as_u64) != Some(1) {
            return Err(invalid("unknown version"));
        }
        let kid = v
            .get("kid")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("no kid"))?;
        let key = keys.key(kid).ok_or_else(|| invalid("unknown key id"))?;
        let sig = Signature::from_slice(&sig_bytes).map_err(|_| invalid("bad signature length"))?;
        key.verify_strict(&payload_bytes, &sig)
            .map_err(|_| invalid("bad signature"))?;
        let time = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .and_then(parse_rfc3339)
                .ok_or_else(|| invalid(&format!("bad `{k}`")))
        };
        let (iat_s, exp_s) = (time("iat")?, time("exp")?);
        let ent = v
            .get("ent")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("no `ent`"))?
            .iter()
            .filter_map(Value::as_str)
            .filter(|e| is_entitlement_id(e))
            .map(str::to_string)
            .collect();
        let state = if now_s <= exp_s {
            TokenState::Valid
        } else if now_s <= exp_s.saturating_add(GRACE_S) {
            TokenState::Grace {
                days_left: (exp_s + GRACE_S - now_s) / 86_400,
            }
        } else {
            return Err(ServiceError::new(
                codes::LICENSE_EXPIRED,
                format!(
                    "a licence expired on {} and its 14-day grace period is over: renew it",
                    format_rfc3339(exp_s)
                ),
            ));
        };
        Ok(LicenseToken {
            raw: raw.to_string(),
            payload: LicensePayload {
                v: 1,
                sub: v
                    .get("sub")
                    .and_then(Value::as_str)
                    .map(|s| sanitize(s, 128))
                    .unwrap_or_default(),
                iat_s,
                exp_s,
                ent,
                dev: v.get("dev").and_then(Value::as_u64),
                kid: kid.to_string(),
            },
            state,
        })
    }
}

/// What the app owns: the union of the valid tokens' entitlements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entitlements {
    ids: BTreeSet<String>,
}

/// The result of checking a set of tokens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LicenseCheck {
    /// The union of the valid tokens' entitlements.
    pub entitlements: Entitlements,
    /// The tokens that passed (valid or in grace).
    pub tokens: Vec<LicenseToken>,
    /// Why each other token was refused (`ferro.license.invalid` /
    /// `ferro.license.expired`).
    pub refused: Vec<ServiceError>,
}

impl LicenseCheck {
    /// Warnings for the user: tokens in grace (days left) and refused ones.
    pub fn warnings(&self) -> Vec<String> {
        let mut w: Vec<String> = self
            .tokens
            .iter()
            .filter_map(|t| match t.state {
                TokenState::Grace { days_left } => Some(format!(
                    "A licence expired on {}: it keeps working {days_left} more day(s); renew it",
                    format_rfc3339(t.payload.exp_s)
                )),
                TokenState::Valid => None,
            })
            .collect();
        w.extend(self.refused.iter().map(|e| e.to_string()));
        w
    }
}

impl Entitlements {
    /// None owned.
    pub fn none() -> Entitlements {
        Entitlements::default()
    }

    /// Check every token in `raws` at `now_s` and add up the entitlements
    /// of those that pass.
    pub fn check(raws: &[String], keys: &KeyRing, now_s: i64) -> LicenseCheck {
        let mut out = LicenseCheck::default();
        for raw in raws {
            match LicenseToken::verify(raw, keys, now_s) {
                Ok(t) => {
                    out.entitlements.ids.extend(t.payload.ent.iter().cloned());
                    out.tokens.push(t);
                }
                Err(e) => out.refused.push(e),
            }
        }
        out
    }

    /// True when `id` (`plugin:acme.stairs`, `cloud.sync`) is owned.
    pub fn has(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// The owned ids, sorted.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.ids.iter().map(String::as_str)
    }

    /// True when nothing is owned.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

fn tokens_of(v: &Value) -> Result<Vec<String>, ServiceError> {
    let list = v
        .get("tokens")
        .and_then(Value::as_array)
        .ok_or_else(|| ServiceError::protocol("the answer has no `tokens` list"))?;
    Ok(list
        .iter()
        .filter_map(Value::as_str)
        .filter(|t| !t.is_empty() && t.len() <= MAX_TOKEN_BYTES)
        .take(MAX_TOKENS)
        .map(str::to_string)
        .collect())
}

/// The signed-in account's tokens: `GET /v1/licenses`. Fails with the
/// `ferro.service.*` codes.
pub fn fetch_licenses(
    client: &ServiceClient,
    access: &Secret,
) -> Result<Vec<String>, ServiceError> {
    tokens_of(&client.get("/v1/licenses", Some(access))?.into_success()?)
}

/// Redeem an activation code bought on the web:
/// `POST /v1/licenses/activate {code}` (sent signed in when `access` is
/// given). Fails with the `ferro.service.*` codes; an unknown code is
/// `ferro.service.protocol` with the server's message.
pub fn activate(
    client: &ServiceClient,
    code: &str,
    access: Option<&Secret>,
) -> Result<Vec<String>, ServiceError> {
    let code = code.trim();
    if code.is_empty() || code.len() > 256 {
        return Err(ServiceError::protocol("type the activation code"));
    }
    tokens_of(
        &client
            .post("/v1/licenses/activate", &json!({ "code": code }), access)?
            .into_success()?,
    )
}

/// The cached tokens (`licenses.json`); empty when missing or broken.
pub fn load_cache(home: &Path) -> Vec<String> {
    std::fs::read_to_string(home.join(LICENSES_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| tokens_of(&v).ok())
        .unwrap_or_default()
}

/// Write the token cache (deduplicated, at most [`MAX_TOKENS`]).
pub fn save_cache(home: &Path, tokens: &[String]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    let list: Vec<&String> = tokens
        .iter()
        .filter(|t| seen.insert(t.as_str()))
        .take(MAX_TOKENS)
        .collect();
    let text =
        serde_json::to_string_pretty(&json!({ "tokens": list })).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    std::fs::write(home.join(LICENSES_FILE), text)
        .map_err(|e| format!("cannot write {LICENSES_FILE}: {e}"))
}

/// Merge tokens fetched for `account_id` into the cache: cached tokens of
/// that account are replaced (so revoked ones go), others (activation
/// codes of no or another account) are kept. Returns the new list.
pub fn merge_fetched(
    cached: &[String],
    fetched: &[String],
    account_id: &str,
    keys: &KeyRing,
    now_s: i64,
) -> Vec<String> {
    let mut out: Vec<String> = cached
        .iter()
        .filter(|raw| match LicenseToken::verify(raw, keys, now_s) {
            Ok(t) => t.payload.sub != account_id,
            Err(_) => false,
        })
        .cloned()
        .collect();
    for f in fetched {
        if !out.contains(f) {
            out.push(f.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::b64url_encode;
    use ed25519_dalek::{Signer, SigningKey};

    /// A test-only key, created here at test time.
    fn test_key() -> SigningKey {
        let mut seed = [0u8; 32];
        seed[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        seed[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        SigningKey::from_bytes(&seed)
    }

    fn token(key: &SigningKey, payload: &Value) -> String {
        let bytes = serde_json::to_vec(payload).unwrap();
        let sig = key.sign(&bytes);
        format!(
            "{}.{}",
            b64url_encode(&bytes),
            b64url_encode(&sig.to_bytes())
        )
    }

    fn payload(exp: &str, ent: &[&str]) -> Value {
        json!({"v": 1, "sub": "u1", "iat": "2026-01-01T00:00:00Z", "exp": exp,
               "ent": ent, "dev": 3, "kid": "t1"})
    }

    fn ring(key: &SigningKey) -> KeyRing {
        KeyRing::empty().with_key("t1", &key.verifying_key().to_bytes(), true)
    }

    #[test]
    fn valid_token_gives_its_entitlements() {
        let k = test_key();
        let t = token(
            &k,
            &payload(
                "2027-01-01T00:00:00Z",
                &["plugin:acme.stairs", "cloud.sync", "BAD ID"],
            ),
        );
        let now = parse_rfc3339("2026-11-24T00:00:00Z").unwrap();
        let v = LicenseToken::verify(&t, &ring(&k), now).unwrap();
        assert_eq!(v.state, TokenState::Valid);
        assert_eq!(v.payload.ent, vec!["plugin:acme.stairs", "cloud.sync"]);
        assert_eq!(v.payload.dev, Some(3));
        let c = Entitlements::check(std::slice::from_ref(&t), &ring(&k), now);
        assert!(c.entitlements.has("cloud.sync") && !c.entitlements.has("render.cloud"));
        assert!(c.warnings().is_empty());
    }

    #[test]
    fn tampered_wrong_key_and_malformed_tokens_are_invalid() {
        let k = test_key();
        let t = token(&k, &payload("2027-01-01T00:00:00Z", &["cloud.sync"]));
        let now = parse_rfc3339("2026-11-24T00:00:00Z").unwrap();
        // Payload changed after signing: entitlement added.
        let forged = b64url_encode(
            &serde_json::to_vec(&payload(
                "2027-01-01T00:00:00Z",
                &["cloud.sync", "enterprise"],
            ))
            .unwrap(),
        );
        let sig = t.split_once('.').unwrap().1;
        let tampered = format!("{forged}.{sig}");
        let other = test_key();
        let mut bad_v = payload("2027-01-01T00:00:00Z", &[]);
        bad_v["v"] = json!(2);
        let mut bad_kid = payload("2027-01-01T00:00:00Z", &[]);
        bad_kid["kid"] = json!("nope");
        for bad in [
            tampered,
            token(&other, &payload("2027-01-01T00:00:00Z", &["cloud.sync"])),
            token(&k, &bad_v),
            token(&k, &bad_kid),
            token(
                &k,
                &json!({"v": 1, "kid": "t1", "iat": "x", "exp": "y", "ent": []}),
            ),
            "garbage".into(),
            "a.b".into(),
            String::new(),
            format!("{}.", t.split_once('.').unwrap().0),
        ] {
            let e = LicenseToken::verify(&bad, &ring(&k), now).unwrap_err();
            assert_eq!(e.code, codes::LICENSE_INVALID, "{bad}: {e}");
        }
        assert!(LicenseToken::verify(&t, &KeyRing::empty(), now).is_err());
    }

    #[test]
    fn expired_token_works_in_grace_then_is_refused() {
        let k = test_key();
        let t = token(&k, &payload("2026-11-01T00:00:00Z", &["plugin:p"]));
        let exp = parse_rfc3339("2026-11-01T00:00:00Z").unwrap();
        let r = ring(&k);
        // At exp exactly: valid. One second later: grace with 13 full days left.
        assert_eq!(
            LicenseToken::verify(&t, &r, exp).unwrap().state,
            TokenState::Valid
        );
        assert_eq!(
            LicenseToken::verify(&t, &r, exp + 1).unwrap().state,
            TokenState::Grace { days_left: 13 }
        );
        // Last second of grace: still works.
        let last = LicenseToken::verify(&t, &r, exp + GRACE_S).unwrap();
        assert_eq!(last.state, TokenState::Grace { days_left: 0 });
        let c = Entitlements::check(std::slice::from_ref(&t), &r, exp + 86_400);
        assert!(c.entitlements.has("plugin:p"));
        assert!(c.warnings()[0].contains("renew"));
        // One second after the grace period: refused.
        let e = LicenseToken::verify(&t, &r, exp + GRACE_S + 1).unwrap_err();
        assert_eq!(e.code, codes::LICENSE_EXPIRED);
        let c = Entitlements::check(&[t], &r, exp + GRACE_S + 1);
        assert!(c.entitlements.is_empty());
        assert_eq!(c.refused[0].code, codes::LICENSE_EXPIRED);
    }

    #[test]
    fn development_key_only_for_localhost() {
        let k = test_key();
        let pk = b64url_encode(&k.verifying_key().to_bytes());
        let local = ServiceConfig::online("http://127.0.0.1:9", None).unwrap();
        let remote = ServiceConfig::online("https://services.ferro.example", None).unwrap();
        assert!(KeyRing::builtin_with(&local, Some(&pk)).is_development(DEVELOPMENT_KID));
        assert!(KeyRing::builtin_with(&remote, Some(&pk))
            .key(DEVELOPMENT_KID)
            .is_none());
        assert!(KeyRing::builtin_with(&local, Some("not a key")).is_empty());
        assert!(KeyRing::builtin_with(&ServiceConfig::offline(None), Some(&pk)).is_empty());
    }

    #[test]
    fn cache_round_trips_and_merge_replaces_the_accounts_tokens() {
        let k = test_key();
        let r = ring(&k);
        let now = parse_rfc3339("2026-11-24T00:00:00Z").unwrap();
        let mine_old = token(&k, &payload("2027-01-01T00:00:00Z", &["plugin:old"]));
        let mut other = payload("2027-01-01T00:00:00Z", &["plugin:code"]);
        other["sub"] = json!("");
        let activation = token(&k, &other);
        let mine_new = token(&k, &payload("2027-06-01T00:00:00Z", &["plugin:new"]));
        let merged = merge_fetched(
            &[mine_old.clone(), activation.clone(), "junk".into()],
            std::slice::from_ref(&mine_new),
            "u1",
            &r,
            now,
        );
        assert_eq!(merged, vec![activation.clone(), mine_new.clone()]);
        let home =
            std::env::temp_dir().join(format!("ferro-lic-{}", uuid::Uuid::new_v4().simple()));
        assert!(load_cache(&home).is_empty());
        save_cache(
            &home,
            &[merged[0].clone(), merged[0].clone(), merged[1].clone()],
        )
        .unwrap();
        assert_eq!(load_cache(&home), merged);
        let _ = std::fs::remove_dir_all(home);
    }
}
