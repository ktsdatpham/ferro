//! Marketplace client (contract §4): catalog search, a [`Listing`], plugin
//! download with a SHA-256 check, checkout and orders.
//!
//! Everything the server sends is checked before use: ids must be safe in
//! a URL path ([`crate::text::is_safe_id`]), text is sanitised for display,
//! URLs (download, checkout, screenshots) must be https (or localhost), and
//! a download whose SHA-256 differs from the listing's is deleted and
//! refused with `ferro.market.checksum`. The zip then goes through the
//! plugin installer's zip-slip-safe path (`ferro_mcp::plugins`).

use crate::account::Secret;
use crate::config::check_url;
use crate::http::ServiceClient;
use crate::text::{hex, is_safe_id, query_encode, sanitize};
use crate::{codes, ServiceError};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Largest plugin download (bytes), like the installer's limit.
pub const MAX_DOWNLOAD_BYTES: u64 = 200 * 1024 * 1024;

/// Who publishes a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publisher {
    /// Publisher id.
    pub id: String,
    /// Display name (sanitised).
    pub name: String,
    /// Checked by the marketplace.
    pub verified: bool,
}

/// A plugin's price, in đồng (VND, no decimals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Price {
    /// `{"kind": "free"}`.
    Free,
    /// `{"kind": "one_time", "amount": 490000}`.
    OneTime {
        /// Amount in đồng.
        amount_vnd: u64,
    },
    /// `{"kind": "subscription", "amount": 99000, "period": "month"}`.
    Subscription {
        /// Amount in đồng per period.
        amount_vnd: u64,
        /// `month` or `year`.
        period: String,
    },
}

/// `1234567` → `1,234,567`.
fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Price {
    /// Parse the contract's price object. `None` when malformed.
    pub fn from_json(v: &Value) -> Option<Price> {
        let amount = || v.get("amount").and_then(Value::as_u64);
        match v.get("kind").and_then(Value::as_str)? {
            "free" => Some(Price::Free),
            "one_time" => Some(Price::OneTime {
                amount_vnd: amount()?,
            }),
            "subscription" => {
                let period = v.get("period").and_then(Value::as_str)?;
                if !matches!(period, "month" | "year") {
                    return None;
                }
                Some(Price::Subscription {
                    amount_vnd: amount()?,
                    period: period.to_string(),
                })
            }
            _ => None,
        }
    }

    /// True for [`Price::Free`] (or an amount of 0).
    pub fn is_free(&self) -> bool {
        match self {
            Price::Free => true,
            Price::OneTime { amount_vnd } | Price::Subscription { amount_vnd, .. } => {
                *amount_vnd == 0
            }
        }
    }

    /// Text for the user: `Free`, `490,000 VND`, `99,000 VND / month`.
    pub fn label(&self) -> String {
        match self {
            Price::Free => "Free".into(),
            Price::OneTime { amount_vnd } => format!("{} VND", grouped(*amount_vnd)),
            Price::Subscription { amount_vnd, period } => {
                format!("{} VND / {period}", grouped(*amount_vnd))
            }
        }
    }
}

/// A plugin in the marketplace catalog (contract §4), checked and
/// sanitised.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    /// Plugin id (safe in a URL path).
    pub id: String,
    /// Name.
    pub name: String,
    /// Who publishes it.
    pub publisher: Publisher,
    /// One-line summary.
    pub summary: String,
    /// Version offered.
    pub version: String,
    /// Price.
    pub price: Price,
    /// Entitlement that unlocks it (`plugin:<id>` when the server gives
    /// none for a paid plugin; empty for a free plugin without one).
    pub entitlement: String,
    /// Where the zip is (https, or localhost).
    pub download_url: String,
    /// Expected SHA-256 of the zip, lower-case hex.
    pub sha256: String,
    /// Zip size in bytes (informative).
    pub size: u64,
    /// Screenshot URLs (refused ones dropped).
    pub screenshots: Vec<String>,
    /// Average rating 0 to 5, when rated.
    pub rating: Option<f64>,
    /// Number of installs.
    pub installs: u64,
}

impl Listing {
    /// Parse and check a listing. Fails with `ferro.service.protocol` when
    /// the id is unsafe, a required field is missing, the price is
    /// malformed, `sha256` is not 64 hex digits or `download_url` is
    /// refused.
    pub fn from_json(v: &Value) -> Result<Listing, ServiceError> {
        let bad = |why: &str| ServiceError::protocol(format!("bad listing: {why}"));
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| is_safe_id(s))
            .ok_or_else(|| bad("id"))?
            .to_string();
        let text = |k: &str, max: usize| {
            v.get(k)
                .and_then(Value::as_str)
                .map(|s| sanitize(s, max))
                .unwrap_or_default()
        };
        let p = v.get("publisher").cloned().unwrap_or(Value::Null);
        let publisher = Publisher {
            id: p
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| is_safe_id(s))
                .unwrap_or("")
                .to_string(),
            name: p
                .get("name")
                .and_then(Value::as_str)
                .map(|s| sanitize(s, 80))
                .unwrap_or_default(),
            verified: p.get("verified").and_then(Value::as_bool).unwrap_or(false),
        };
        let price = v
            .get("price")
            .and_then(Price::from_json)
            .ok_or_else(|| bad("price"))?;
        let mut entitlement = text("entitlement", 128);
        if entitlement.is_empty() && !price.is_free() {
            entitlement = format!("plugin:{id}");
        }
        let download_url = v
            .get("download_url")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("download_url"))?
            .to_string();
        check_url(&download_url)?;
        let sha256 = v
            .get("sha256")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| bad("sha256"))?;
        let screenshots = v
            .get("screenshots")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|u| check_url(u).is_ok())
                    .take(10)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let name = match text("name", 80) {
            n if n.is_empty() => id.clone(),
            n => n,
        };
        Ok(Listing {
            name,
            publisher,
            summary: text("summary", 300),
            version: text("version", 40),
            price,
            entitlement,
            download_url,
            sha256,
            size: v.get("size").and_then(Value::as_u64).unwrap_or(0),
            screenshots,
            rating: v
                .get("rating")
                .and_then(Value::as_f64)
                .filter(|r| r.is_finite())
                .map(|r| r.clamp(0.0, 5.0)),
            installs: v.get("installs").and_then(Value::as_u64).unwrap_or(0),
            id,
        })
    }

    /// True when it costs nothing.
    pub fn is_free(&self) -> bool {
        self.price.is_free()
    }
}

/// One page of catalog results.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogPage {
    /// The listings (malformed ones skipped).
    pub plugins: Vec<Listing>,
    /// Listings the server sent that were skipped as malformed.
    pub skipped: usize,
    /// The `page` value of the next page, if any.
    pub next: Option<String>,
}

/// Search the catalog: `GET /v1/market/plugins?query=&page=`.
pub fn search(
    client: &ServiceClient,
    query: &str,
    page: Option<&str>,
) -> Result<CatalogPage, ServiceError> {
    let mut path = format!("/v1/market/plugins?query={}", query_encode(query.trim()));
    if let Some(p) = page {
        path.push_str(&format!("&page={}", query_encode(p)));
    }
    let v = client.get(&path, None)?.into_success()?;
    let list = v
        .get("plugins")
        .and_then(Value::as_array)
        .ok_or_else(|| ServiceError::protocol("the answer has no `plugins` list"))?;
    let mut plugins = Vec::new();
    let mut skipped = 0;
    for item in list.iter().take(200) {
        match Listing::from_json(item) {
            Ok(l) => plugins.push(l),
            Err(_) => skipped += 1,
        }
    }
    let next = match v.get("next") {
        Some(Value::String(s)) if is_safe_id(s) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    Ok(CatalogPage {
        plugins,
        skipped,
        next,
    })
}

/// One listing: `GET /v1/market/plugins/{id}`. Fails with
/// `ferro.service.protocol` for an unsafe id or a malformed listing.
pub fn listing(client: &ServiceClient, id: &str) -> Result<Listing, ServiceError> {
    if !is_safe_id(id) {
        return Err(ServiceError::protocol("bad plugin id"));
    }
    let v = client
        .get(&format!("/v1/market/plugins/{id}"), None)?
        .into_success()?;
    Listing::from_json(&v)
}

/// Download a listing's zip into `dir` (as `<id>.zip`) and check its
/// SHA-256. Returns the zip's path, ready for the plugin installer. Fails
/// with `ferro.market.checksum` (the file is deleted) when the digest
/// differs, or like [`ServiceClient::download`].
pub fn download(
    client: &ServiceClient,
    listing: &Listing,
    dir: &Path,
) -> Result<PathBuf, ServiceError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| ServiceError::protocol(format!("cannot create {}: {e}", dir.display())))?;
    let dest = dir.join(format!("{}.zip", listing.id));
    let (_, sha) = client.download(&listing.download_url, MAX_DOWNLOAD_BYTES, &dest)?;
    let got = hex(&sha);
    if got != listing.sha256 {
        let _ = std::fs::remove_file(&dest);
        return Err(ServiceError::new(
            codes::MARKET_CHECKSUM,
            format!(
                "the download of {} does not match its checksum; nothing was installed",
                listing.name
            ),
        ));
    }
    Ok(dest)
}

/// A started purchase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    /// The payment page to open in the browser (checked).
    pub checkout_url: String,
    /// The order to poll (safe id).
    pub order_id: String,
}

/// Start buying a plugin: `POST /v1/market/checkout {plugin_id}` (signed
/// in). Fails with `ferro.service.unauthorized` when not signed in and
/// `ferro.service.protocol` when the URL or order id is refused.
pub fn checkout(
    client: &ServiceClient,
    access: &Secret,
    plugin_id: &str,
) -> Result<Checkout, ServiceError> {
    if !is_safe_id(plugin_id) {
        return Err(ServiceError::protocol("bad plugin id"));
    }
    let v = client
        .post(
            "/v1/market/checkout",
            &json!({ "plugin_id": plugin_id }),
            Some(access),
        )?
        .into_success()?;
    let checkout_url = v
        .get("checkout_url")
        .and_then(Value::as_str)
        .ok_or_else(|| ServiceError::protocol("no `checkout_url`"))?
        .to_string();
    check_url(&checkout_url)?;
    let order_id = v
        .get("order_id")
        .and_then(Value::as_str)
        .filter(|s| is_safe_id(s))
        .ok_or_else(|| ServiceError::protocol("no valid `order_id`"))?
        .to_string();
    Ok(Checkout {
        checkout_url,
        order_id,
    })
}

/// Where an order stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    /// Not paid yet.
    Pending,
    /// Paid: refresh the licences.
    Paid,
    /// The payment failed.
    Failed,
    /// The buyer cancelled.
    Cancelled,
}

impl OrderStatus {
    /// The contract spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            OrderStatus::Pending => "pending",
            OrderStatus::Paid => "paid",
            OrderStatus::Failed => "failed",
            OrderStatus::Cancelled => "cancelled",
        }
    }

    /// True once the order will not change any more.
    pub fn is_final(self) -> bool {
        self != OrderStatus::Pending
    }
}

/// An order's status: `GET /v1/market/orders/{order_id}` (signed in).
pub fn order_status(
    client: &ServiceClient,
    access: &Secret,
    order_id: &str,
) -> Result<OrderStatus, ServiceError> {
    if !is_safe_id(order_id) {
        return Err(ServiceError::protocol("bad order id"));
    }
    let v = client
        .get(&format!("/v1/market/orders/{order_id}"), Some(access))?
        .into_success()?;
    match v.get("status").and_then(Value::as_str) {
        Some("pending") => Ok(OrderStatus::Pending),
        Some("paid") => Ok(OrderStatus::Paid),
        Some("failed") => Ok(OrderStatus::Failed),
        Some("cancelled") => Ok(OrderStatus::Cancelled),
        _ => Err(ServiceError::protocol("unknown order status")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing_json() -> Value {
        json!({
            "id": "acme.stairs", "name": "Stairs\u{202E}", "summary": "Stairs\nby code",
            "publisher": {"id": "acme", "name": "ACME", "verified": true},
            "version": "1.2.0", "price": {"kind": "one_time", "amount": 490000},
            "download_url": "https://cdn.example/acme.zip",
            "sha256": "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
            "size": 3, "screenshots": ["https://cdn.example/1.png", "http://evil.example/x.png"],
            "rating": 9.0, "installs": 12
        })
    }

    #[test]
    fn listing_is_checked_and_sanitised() {
        let l = Listing::from_json(&listing_json()).unwrap();
        assert_eq!(l.name, "Stairs");
        assert_eq!(l.summary, "Stairs by code");
        assert_eq!(l.entitlement, "plugin:acme.stairs");
        assert_eq!(l.sha256.len(), 64);
        assert_eq!(l.sha256, l.sha256.to_ascii_lowercase());
        assert_eq!(l.screenshots.len(), 1);
        assert_eq!(l.rating, Some(5.0));
        assert_eq!(l.price.label(), "490,000 VND");
        assert!(!l.is_free());
        for (k, v) in [
            ("id", json!("../etc")),
            ("price", json!({"kind": "gift"})),
            ("download_url", json!("http://cdn.example/a.zip")),
            ("sha256", json!("abc")),
        ] {
            let mut j = listing_json();
            j[k] = v;
            assert_eq!(
                Listing::from_json(&j).unwrap_err().code,
                codes::PROTOCOL,
                "{k}"
            );
        }
    }

    #[test]
    fn prices_parse_and_print() {
        assert_eq!(
            Price::from_json(&json!({"kind": "free"})),
            Some(Price::Free)
        );
        let s =
            Price::from_json(&json!({"kind": "subscription", "amount": 99000, "period": "month"}))
                .unwrap();
        assert_eq!(s.label(), "99,000 VND / month");
        assert!(
            Price::from_json(&json!({"kind": "subscription", "amount": 1, "period": "day"}))
                .is_none()
        );
        assert!(Price::from_json(&json!({"kind": "one_time", "amount": -1})).is_none());
        assert!(Price::OneTime { amount_vnd: 0 }.is_free());
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(grouped(100), "100");
    }

    #[test]
    fn offline_market_calls_fail_offline() {
        let c = ServiceClient::new(crate::ServiceConfig::offline(None));
        assert_eq!(search(&c, "x", None).unwrap_err().code, codes::OFFLINE);
        assert_eq!(listing(&c, "a").unwrap_err().code, codes::OFFLINE);
        assert_eq!(listing(&c, "../a").unwrap_err().code, codes::PROTOCOL);
        let t = Secret::new("t");
        assert_eq!(checkout(&c, &t, "a").unwrap_err().code, codes::OFFLINE);
        assert_eq!(
            order_status(&c, &t, "a/b").unwrap_err().code,
            codes::PROTOCOL
        );
    }
}
