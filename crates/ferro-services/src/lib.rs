//! Client of the open Ferro services contract (ADR 0010,
//! `docs/services/README.md`). This is the only crate that does network I/O
//! for services; `ferro-core` never depends on it and an open build works
//! fully offline without it ever touching the network.
//!
//! - [`config`]: `services.json`, `FERRO_SERVICES_URL`, offline mode, the
//!   TLS rule (no `http://` except to the local machine).
//! - [`http`]: a small blocking JSON client with timeouts
//!   ([`http::ServiceClient`]).
//! - [`account`]: sign-in by the OAuth device flow (RFC 8628), refresh,
//!   revoke, token storage in `account.json`.
//! - [`license`]: licence tokens (Ed25519, key list by `kid`, 14-day
//!   grace), [`license::Entitlements`], the token cache.
//! - [`market`]: catalog search, listings, plugin download with a SHA-256
//!   check, checkout and orders.
//! - [`mock`]: an in-memory server implementing the contract on
//!   `127.0.0.1`, for tests and local development (`ferro-services-mock`).
//!
//! Every failure is a [`ServiceError`] with a stable code from [`codes`].
//!
//! wasm: this crate does blocking socket I/O, so it is not built for
//! `wasm32-unknown-unknown`; the wasm CI job builds `ferro-core` only, like
//! for `ferro-kernel`.

pub mod account;
pub mod config;
pub mod http;
pub mod license;
pub mod market;
pub mod mock;
pub mod text;

pub use account::{Account, AccountTokens, Secret, SignedIn};
pub use config::ServiceConfig;
pub use http::ServiceClient;
pub use license::{Entitlements, KeyRing, LicenseToken};
pub use market::{Listing, Price};

/// The stable failure codes of the services client (contract §7). Once
/// released they are never renamed.
pub mod codes {
    /// Services are switched off (`services.json` `enabled: false` or an
    /// empty `base_url`); no network was touched.
    pub const OFFLINE: &str = "ferro.service.offline";
    /// The server did not answer (connection, DNS, timeout) or answered 5xx.
    pub const UNAVAILABLE: &str = "ferro.service.unavailable";
    /// Not signed in, or the access token expired and could not be refreshed.
    pub const UNAUTHORIZED: &str = "ferro.service.unauthorized";
    /// The answer did not follow the contract (not JSON, missing fields, a
    /// refused URL, a device code that expired or was denied).
    pub const PROTOCOL: &str = "ferro.service.protocol";
    /// A licence token failed verification (format, `v`, `kid`, signature).
    pub const LICENSE_INVALID: &str = "ferro.license.invalid";
    /// A licence token is past its 14-day grace period.
    pub const LICENSE_EXPIRED: &str = "ferro.license.expired";
    /// The entitlement for this plugin or feature is not owned.
    pub const LICENSE_MISSING: &str = "ferro.license.missing";
    /// A downloaded plugin does not match its `sha256`.
    pub const MARKET_CHECKSUM: &str = "ferro.market.checksum";
}

/// A failure of a service call or check: a stable `code` from [`codes`]
/// and a message for the user (never containing a token).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceError {
    /// One of the [`codes`].
    pub code: &'static str,
    /// What went wrong, for the user.
    pub message: String,
}

impl ServiceError {
    /// A failure with this code and message.
    pub fn new(code: &'static str, message: impl Into<String>) -> ServiceError {
        ServiceError {
            code,
            message: message.into(),
        }
    }

    /// `ferro.service.offline`.
    pub fn offline() -> ServiceError {
        ServiceError::new(codes::OFFLINE, "services are switched off (offline mode)")
    }

    /// `ferro.service.protocol` with this message.
    pub fn protocol(message: impl Into<String>) -> ServiceError {
        ServiceError::new(codes::PROTOCOL, message)
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ServiceError {}

/// The Ferro folder (`$FERRO_HOME`, else `~/.ferro`), where
/// `services.json`, `account.json` and `licenses.json` live. `None` when
/// no home folder is known.
pub fn ferro_dir() -> Option<std::path::PathBuf> {
    ferro_tools::link::ferro_dir()
}
