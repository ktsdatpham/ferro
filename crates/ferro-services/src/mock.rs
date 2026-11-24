//! An in-memory server implementing the services contract, for tests and
//! local development. Open, fake payments, `127.0.0.1` only.
//!
//! - Device flow: approves by itself after [`MockOptions::approve_after_polls`]
//!   polls (optionally answering `slow_down` once first).
//! - Licences: signed with a key created when the server starts (the
//!   DEVELOPMENT key; its public half is [`MockServer::public_key_b64`]).
//!   Activation code [`ACTIVATION_CODE`] gives `cloud.sync`.
//! - Catalog: [`FREE_PLUGIN`] (free) and [`PAID_PLUGIN`] (490,000 VND,
//!   entitlement `plugin:mock-paid`), zips built in memory.
//! - Checkout: the order becomes paid when its fake payment page
//!   `/pay/<order_id>` is opened.
//!
//! Run it: `cargo run -p ferro-services --bin ferro-services-mock`; it
//! prints the environment to start Ferro against it.

use crate::text::{b64url_encode, format_rfc3339, hex, now_s};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Id of the free plugin in the mock catalog.
pub const FREE_PLUGIN: &str = "mock-free";
/// Id of the paid plugin in the mock catalog.
pub const PAID_PLUGIN: &str = "mock-paid";
/// The activation code the mock accepts (gives `cloud.sync`).
pub const ACTIVATION_CODE: &str = "FERRO-TEST-CODE";
/// The account the mock signs everybody in as.
pub const MOCK_ACCOUNT_ID: &str = "mock-user";
/// Price of the paid plugin (đồng).
pub const PAID_PRICE_VND: u64 = 490_000;

/// How the mock behaves.
#[derive(Debug, Clone)]
pub struct MockOptions {
    /// Polls answered `authorization_pending` before the sign-in succeeds.
    pub approve_after_polls: u32,
    /// Answer `slow_down` to the first poll.
    pub slow_down_once: bool,
    /// Access token lifetime (seconds).
    pub access_lifetime_s: i64,
    /// Licence token lifetime (seconds).
    pub license_lifetime_s: i64,
    /// Port on 127.0.0.1 (0: any free port).
    pub port: u16,
}

impl Default for MockOptions {
    fn default() -> Self {
        MockOptions {
            approve_after_polls: 2,
            slow_down_once: false,
            access_lifetime_s: 3600,
            license_lifetime_s: 365 * 86_400,
            port: 0,
        }
    }
}

#[derive(Default)]
struct State {
    device_codes: HashMap<String, u32>,
    access: HashMap<String, i64>,
    refresh: BTreeSet<String>,
    owned: BTreeSet<String>,
    orders: HashMap<String, (String, &'static str)>,
    corrupt: BTreeSet<String>,
    polls: u32,
    requests: u32,
}

struct Shared {
    url: String,
    key: SigningKey,
    opts: MockOptions,
    zips: HashMap<String, Vec<u8>>,
    state: Mutex<State>,
}

/// A running mock server; stops when dropped.
pub struct MockServer {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    addr: SocketAddr,
}

fn random32() -> [u8; 32] {
    let mut b = [0u8; 32];
    b[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    b[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    b
}

fn random_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

const CRC_POLY: u32 = 0xEDB8_8320;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ CRC_POLY
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A zip (stored, no compression) of `files` (`path`, contents).
pub fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let (n, len) = (name.len() as u16, data.len() as u32);
        let mut head = Vec::new();
        head.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        head.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, flags, method 0, time, date
        head.extend_from_slice(&crc.to_le_bytes());
        head.extend_from_slice(&len.to_le_bytes());
        head.extend_from_slice(&len.to_le_bytes());
        head.extend_from_slice(&n.to_le_bytes());
        head.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&head);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&n.to_le_bytes());
        central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // extra, comment, disk, attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    let count = files.len() as u16;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn plugin_zip(id: &str, paid: bool) -> Vec<u8> {
    let mut manifest = json!({
        "id": id,
        "name": if paid { "Mock paid tools" } else { "Mock free tools" },
        "version": "1.0.0",
        "author": "Ferro mock",
        "description": "A plugin served by the Ferro mock services server.",
        "publisher": "ferro-mock",
        "commands": [{"id": "hello", "label": if paid { "Paid hello" } else { "Free hello" },
                      "group": "menu", "run": "main:hello"}]
    });
    if paid {
        manifest["price"] = json!({"kind": "one_time", "amount": PAID_PRICE_VND});
        manifest["entitlement"] = json!(format!("plugin:{id}"));
    } else {
        manifest["price"] = json!({"kind": "free"});
    }
    let main = format!("def hello():\n    print(\"hello from {id}\")\n    return \"{id} ok\"\n");
    let manifest = serde_json::to_vec_pretty(&manifest).unwrap_or_default();
    zip_stored(&[
        (&format!("{id}/plugin.json"), &manifest),
        (&format!("{id}/main.py"), main.as_bytes()),
    ])
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    bearer: Option<String>,
    body: Value,
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
                {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut len = 0usize;
    let mut chunked = false;
    let mut bearer = None;
    let mut header_bytes = 0;
    loop {
        let mut h = String::new();
        let n = reader.read_line(&mut h).ok()?;
        header_bytes += n;
        if n == 0 || header_bytes > 64 * 1024 {
            return None;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let (k, v) = h.split_once(':')?;
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            "content-length" => len = v.parse().ok()?,
            "transfer-encoding" => chunked = v.eq_ignore_ascii_case("chunked"),
            "authorization" => bearer = v.strip_prefix("Bearer ").map(str::to_string),
            _ => {}
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size).ok()?;
            let n = usize::from_str_radix(size.trim().split(';').next()?, 16).ok()?;
            if body.len() + n > 1024 * 1024 {
                return None;
            }
            let mut chunk = vec![0u8; n + 2];
            reader.read_exact(&mut chunk).ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
    } else {
        if len > 1024 * 1024 {
            return None;
        }
        body.resize(len, 0);
        reader.read_exact(&mut body).ok()?;
    }
    let (path, q) = target.split_once('?').unwrap_or((&target, ""));
    let query = q
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect();
    Some(Request {
        method,
        path: path.to_string(),
        query,
        bearer,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    })
}

enum Answer {
    Json(u16, Value),
    Bytes(u16, &'static str, Vec<u8>),
}

fn write_answer(stream: &mut TcpStream, a: Answer) {
    let (status, ctype, body) = match a {
        Answer::Json(s, v) => (s, "application/json", v.to_string().into_bytes()),
        Answer::Bytes(s, t, b) => (s, t, b),
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

impl Shared {
    fn sign(&self, sub: &str, ent: &[String]) -> String {
        let now = now_s();
        let payload = json!({
            "v": 1, "sub": sub, "iat": format_rfc3339(now),
            "exp": format_rfc3339(now + self.opts.license_lifetime_s),
            "ent": ent, "dev": 3, "kid": crate::license::DEVELOPMENT_KID,
        });
        let bytes = serde_json::to_vec(&payload).unwrap_or_default();
        let sig = self.key.sign(&bytes);
        format!(
            "{}.{}",
            b64url_encode(&bytes),
            b64url_encode(&sig.to_bytes())
        )
    }

    fn listing(&self, id: &str) -> Option<Value> {
        let zip = self.zips.get(id)?;
        let paid = id == PAID_PLUGIN;
        let mut l = json!({
            "id": id,
            "name": if paid { "Mock paid tools" } else { "Mock free tools" },
            "publisher": {"id": "ferro-mock", "name": "Ferro mock", "verified": true},
            "summary": if paid { "A paid plugin for testing Buy." } else { "A free plugin for testing Install." },
            "version": "1.0.0",
            "price": if paid { json!({"kind": "one_time", "amount": PAID_PRICE_VND}) } else { json!({"kind": "free"}) },
            "download_url": format!("{}/files/{id}.zip", self.url),
            "sha256": hex(&Sha256::digest(zip)),
            "size": zip.len(),
            "screenshots": [],
            "rating": 4.5,
            "installs": 7,
        });
        if paid {
            l["entitlement"] = json!(format!("plugin:{id}"));
        }
        Some(l)
    }

    fn handle(&self, r: Request) -> Answer {
        let mut st = match self.state.lock() {
            Ok(s) => s,
            Err(p) => p.into_inner(),
        };
        st.requests += 1;
        let now = now_s();
        let signed_in = r
            .bearer
            .as_ref()
            .and_then(|t| st.access.get(t))
            .is_some_and(|exp| *exp > now);
        let unauthorized = Answer::Json(401, json!({"error": "unauthorized"}));
        let body_str = |k: &str| {
            r.body
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        match (r.method.as_str(), r.path.as_str()) {
            ("POST", "/v1/auth/device") => {
                let code = random_id();
                st.device_codes.insert(code.clone(), 0);
                Answer::Json(
                    200,
                    json!({
                        "device_code": code, "user_code": "FERR-0123",
                        "verification_uri": format!("{}/device", self.url),
                        "interval": 1, "expires_in": 600,
                    }),
                )
            }
            ("POST", "/v1/auth/token") => {
                let code = body_str("device_code");
                let slow_down_once = self.opts.slow_down_once;
                let approve = self.opts.approve_after_polls;
                let Some(polls) = st.device_codes.get_mut(&code) else {
                    return Answer::Json(400, json!({"error": "expired_token"}));
                };
                *polls += 1;
                let n = *polls;
                st.polls += 1;
                if slow_down_once && n == 1 {
                    return Answer::Json(400, json!({"error": "slow_down"}));
                }
                if n <= approve {
                    return Answer::Json(400, json!({"error": "authorization_pending"}));
                }
                st.device_codes.remove(&code);
                let (access, refresh) = (random_id(), random_id());
                st.access
                    .insert(access.clone(), now + self.opts.access_lifetime_s);
                st.refresh.insert(refresh.clone());
                Answer::Json(
                    200,
                    json!({
                        "access_token": access, "refresh_token": refresh,
                        "expires_in": self.opts.access_lifetime_s,
                        "account": {"id": MOCK_ACCOUNT_ID, "name": "Mock User", "email": "mock@example.com"},
                    }),
                )
            }
            ("POST", "/v1/auth/refresh") => {
                if !st.refresh.contains(&body_str("refresh_token")) {
                    return Answer::Json(400, json!({"error": "invalid_grant"}));
                }
                let access = random_id();
                st.access
                    .insert(access.clone(), now + self.opts.access_lifetime_s);
                Answer::Json(
                    200,
                    json!({"access_token": access, "expires_in": self.opts.access_lifetime_s}),
                )
            }
            ("POST", "/v1/auth/revoke") => {
                st.refresh.remove(&body_str("refresh_token"));
                if let Some(t) = &r.bearer {
                    st.access.remove(t);
                }
                Answer::Json(200, json!({}))
            }
            ("GET", "/device") => Answer::Bytes(
                200,
                "text/html; charset=utf-8",
                b"<h1>Ferro mock</h1><p>Signed in. Go back to Ferro.</p>".to_vec(),
            ),
            ("GET", "/v1/licenses") => {
                if !signed_in {
                    return unauthorized;
                }
                let ent: Vec<String> = st.owned.iter().cloned().collect();
                let tokens: Vec<String> = if ent.is_empty() {
                    Vec::new()
                } else {
                    vec![self.sign(MOCK_ACCOUNT_ID, &ent)]
                };
                Answer::Json(200, json!({ "tokens": tokens }))
            }
            ("POST", "/v1/licenses/activate") => {
                if body_str("code") != ACTIVATION_CODE {
                    return Answer::Json(
                        404,
                        json!({"error": "unknown_code", "message": "This activation code is not valid."}),
                    );
                }
                Answer::Json(
                    200,
                    json!({"tokens": [self.sign("", &["cloud.sync".to_string()])]}),
                )
            }
            ("GET", "/v1/market/plugins") => {
                let q = r
                    .query
                    .get("query")
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                let list: Vec<Value> = [FREE_PLUGIN, PAID_PLUGIN]
                    .iter()
                    .filter_map(|id| self.listing(id))
                    .filter(|l| q.is_empty() || l.to_string().to_lowercase().contains(&q))
                    .collect();
                Answer::Json(200, json!({"plugins": list, "next": null}))
            }
            ("POST", "/v1/market/checkout") => {
                if !signed_in {
                    return unauthorized;
                }
                let plugin = body_str("plugin_id");
                if self.listing(&plugin).is_none() {
                    return Answer::Json(404, json!({"error": "no such plugin"}));
                }
                let order = random_id();
                st.orders.insert(order.clone(), (plugin, "pending"));
                Answer::Json(
                    200,
                    json!({
                        "checkout_url": format!("{}/pay/{order}", self.url),
                        "order_id": order,
                    }),
                )
            }
            (_, p) => {
                if let Some(id) = p.strip_prefix("/v1/market/plugins/") {
                    return match self.listing(id) {
                        Some(l) => Answer::Json(200, l),
                        None => Answer::Json(404, json!({"error": "no such plugin"})),
                    };
                }
                if let Some(id) = p.strip_prefix("/v1/market/orders/") {
                    if !signed_in {
                        return unauthorized;
                    }
                    return match st.orders.get(id) {
                        Some((_, status)) => Answer::Json(200, json!({"status": status})),
                        None => Answer::Json(404, json!({"error": "no such order"})),
                    };
                }
                if let Some(id) = p.strip_prefix("/pay/") {
                    let Some((plugin, status)) = st.orders.get_mut(id) else {
                        return Answer::Bytes(404, "text/plain", b"no such order".to_vec());
                    };
                    *status = "paid";
                    let ent = format!("plugin:{plugin}");
                    st.owned.insert(ent);
                    return Answer::Bytes(
                        200,
                        "text/html; charset=utf-8",
                        b"<h1>Ferro mock</h1><p>Paid (fake payment). Go back to Ferro.</p>"
                            .to_vec(),
                    );
                }
                if let Some(file) = p.strip_prefix("/files/") {
                    let id = file.trim_end_matches(".zip");
                    return match self.zips.get(id) {
                        Some(z) => {
                            let mut z = z.clone();
                            if st.corrupt.contains(id) {
                                if let Some(last) = z.last_mut() {
                                    *last ^= 0xFF;
                                }
                            }
                            Answer::Bytes(200, "application/zip", z)
                        }
                        None => Answer::Bytes(404, "text/plain", b"no such file".to_vec()),
                    };
                }
                Answer::Json(404, json!({"error": "not_found"}))
            }
        }
    }
}

impl MockServer {
    /// Start on `127.0.0.1` with `opts`. Fails when the port cannot be
    /// bound.
    pub fn start(opts: MockOptions) -> std::io::Result<MockServer> {
        let listener = TcpListener::bind(("127.0.0.1", opts.port))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let zips = [FREE_PLUGIN, PAID_PLUGIN]
            .iter()
            .map(|id| (id.to_string(), plugin_zip(id, *id == PAID_PLUGIN)))
            .collect();
        let shared = Arc::new(Shared {
            url: format!("http://{addr}"),
            key: SigningKey::from_bytes(&random32()),
            opts,
            zips,
            state: Mutex::new(State::default()),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, stop) = (shared.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut s, _)) => {
                            let _ = s.set_nonblocking(false);
                            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                            let a = match read_request(&mut s) {
                                Some(r) => shared.handle(r),
                                None => Answer::Json(400, json!({"error": "bad_request"})),
                            };
                            write_answer(&mut s, a);
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            })
        };
        Ok(MockServer {
            shared,
            stop,
            thread: Some(thread),
            addr,
        })
    }

    /// Base URL (`http://127.0.0.1:<port>`).
    pub fn url(&self) -> String {
        self.shared.url.clone()
    }

    /// The address it listens on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The public half of its licence signing key.
    pub fn public_key(&self) -> [u8; 32] {
        self.shared.key.verifying_key().to_bytes()
    }

    /// [`Self::public_key`] as base64url (the `FERRO_DEV_LICENSE_KEY` value).
    pub fn public_key_b64(&self) -> String {
        b64url_encode(&self.public_key())
    }

    /// A licence token signed by this server for `sub` with `ent`, valid
    /// for [`MockOptions::license_lifetime_s`].
    pub fn sign_license(&self, sub: &str, ent: &[&str]) -> String {
        let ent: Vec<String> = ent.iter().map(|s| s.to_string()).collect();
        self.shared.sign(sub, &ent)
    }

    /// Serve a corrupted zip for this plugin from now on (its listing keeps
    /// the right SHA-256), to test `ferro.market.checksum`.
    pub fn corrupt_download(&self, plugin_id: &str) {
        self.with_state(|s| {
            s.corrupt.insert(plugin_id.to_string());
        });
    }

    /// Make every access token expired (the next signed-in call refreshes).
    pub fn expire_access_tokens(&self) {
        self.with_state(|s| {
            for exp in s.access.values_mut() {
                *exp = 0;
            }
        });
    }

    /// Requests served so far.
    pub fn requests(&self) -> u32 {
        self.with_state(|s| s.requests)
    }

    /// Token polls answered so far.
    pub fn polls(&self) -> u32 {
        self.with_state(|s| s.polls)
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        let mut st = match self.shared.state.lock() {
            Ok(s) => s,
            Err(p) => p.into_inner(),
        };
        f(&mut st)
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_zip_shape() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let z = zip_stored(&[("a/b.txt", b"hi")]);
        assert_eq!(&z[..4], &[0x50, 0x4b, 0x03, 0x04]);
        assert_eq!(&z[z.len() - 22..z.len() - 18], &[0x50, 0x4b, 0x05, 0x06]);
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%20b+c%zz%"), "a b c%zz%");
    }
}
