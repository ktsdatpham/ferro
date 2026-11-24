//! End to end against the mock server (M6-20.6): sign in, browse, install
//! the free plugin, buy the paid one (fake payment), refresh licences, run
//! the paid plugin; plus offline mode, tampered / expired tokens, a wrong
//! SHA-256 and the TLS rule.

use ferro_core::ops::Actor;
use ferro_mcp::plugins::{self, check_entitlement, LICENSE_MISSING};
use ferro_mcp::python::{interpreter_works, PythonConfig, RunControl};
use ferro_services::account::{self, PollStep};
use ferro_services::license::{self, LicenseToken, TokenState, GRACE_S};
use ferro_services::market::{self, OrderStatus};
use ferro_services::mock::{MockOptions, MockServer, ACTIVATION_CODE, FREE_PLUGIN, PAID_PLUGIN};
use ferro_services::text::now_s;
use ferro_services::{codes, Entitlements, KeyRing, ServiceClient, ServiceConfig};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "ferro-services-e2e-{tag}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn keys(mock: &MockServer) -> KeyRing {
    let config = ServiceConfig::online(&mock.url(), None).unwrap();
    KeyRing::builtin_with(&config, Some(&mock.public_key_b64()))
}

fn no_sleep() -> impl FnMut(std::time::Duration) {
    |_| {}
}

#[test]
fn sign_in_browse_install_buy_refresh_and_run_the_paid_plugin() {
    let mock = MockServer::start(MockOptions {
        approve_after_polls: 2,
        slow_down_once: true,
        ..MockOptions::default()
    })
    .unwrap();
    let home = temp("flow");
    let plugins_root = home.join("plugins");
    let client = ServiceClient::new(ServiceConfig::online(&mock.url(), Some(&home)).unwrap());
    let ring = keys(&mock);

    // Sign in (device flow): slow_down, then pending twice, then signed in.
    let start = account::start_device_flow(&client).unwrap();
    assert_eq!(start.user_code, "FERR-0123");
    assert!(start.verification_uri.starts_with(&mock.url()));
    assert_eq!(
        account::poll_token(&client, &start).unwrap(),
        PollStep::SlowDown
    );
    let mut slept = Vec::new();
    let mut sleep = |d| slept.push(d);
    let mut signed =
        account::wait_for_sign_in(&client, &start, &AtomicBool::new(false), &mut sleep)
            .unwrap()
            .expect("not cancelled");
    assert_eq!(signed.account.id, "mock-user");
    assert_eq!(mock.polls(), 3);
    assert_eq!(slept.len(), 2, "one wait before each poll");
    account::save(&home, &signed).unwrap();
    assert_eq!(account::load(&home).as_ref(), Some(&signed));

    // Browse.
    let page = market::search(&client, "", None).unwrap();
    assert_eq!(page.plugins.len(), 2);
    assert_eq!(page.next, None);
    let paid_page = market::search(&client, "paid plugin", None).unwrap();
    assert_eq!(paid_page.plugins.len(), 1);
    let free = market::listing(&client, FREE_PLUGIN).unwrap();
    let paid = market::listing(&client, PAID_PLUGIN).unwrap();
    assert!(free.is_free() && !paid.is_free());
    assert_eq!(paid.entitlement, "plugin:mock-paid");
    assert_eq!(paid.price.label(), "490,000 VND");

    // Install the free plugin through the zip installer.
    let zip = market::download(&client, &free, &home.join("downloads")).unwrap();
    let staged = plugins::stage_install(&zip, &plugins_root).unwrap();
    assert_eq!(staged.manifest.id, FREE_PLUGIN);
    plugins::finish_install(staged, &plugins_root).unwrap();

    // Install the paid plugin: it loads, but without the entitlement its
    // commands are refused with ferro.license.missing.
    let zip = market::download(&client, &paid, &home.join("downloads")).unwrap();
    let staged = plugins::stage_install(&zip, &plugins_root).unwrap();
    assert!(staged.manifest.is_paid());
    plugins::finish_install(staged, &plugins_root).unwrap();
    let list = plugins::scan(&plugins_root);
    assert_eq!(list.iter().filter(|p| p.active().is_some()).count(), 2);
    let paid_plugin = list.iter().find(|p| p.id() == PAID_PLUGIN).unwrap().clone();
    let paid_manifest = paid_plugin.active().unwrap().clone();
    let free_manifest = list
        .iter()
        .find(|p| p.id() == FREE_PLUGIN)
        .unwrap()
        .active()
        .unwrap()
        .clone();
    let mut changed = false;
    let tokens = account::call_signed_in(&client, &mut signed, &mut changed, |t| {
        license::fetch_licenses(&client, t)
    })
    .unwrap();
    assert!(tokens.is_empty(), "nothing bought yet");
    let owned = Entitlements::check(&tokens, &ring, now_s()).entitlements;
    let refused = check_entitlement(&paid_manifest, |e| owned.has(e)).unwrap_err();
    assert_eq!(refused["failures"][0]["code"], LICENSE_MISSING);
    assert_eq!(refused["failures"][0]["plugin"], PAID_PLUGIN);
    assert!(check_entitlement(&free_manifest, |e| owned.has(e)).is_ok());

    // Buy: checkout, the browser "pays" on the fake page, the order is paid.
    let co = market::checkout(&client, &signed.tokens.access_token, PAID_PLUGIN).unwrap();
    assert!(co.checkout_url.starts_with(&mock.url()));
    assert_eq!(
        market::order_status(&client, &signed.tokens.access_token, &co.order_id).unwrap(),
        OrderStatus::Pending
    );
    ureq::get(&co.checkout_url).call().unwrap();
    assert_eq!(
        market::order_status(&client, &signed.tokens.access_token, &co.order_id).unwrap(),
        OrderStatus::Paid
    );

    // Licence refresh, with an expired access token: refreshed on the way.
    mock.expire_access_tokens();
    let old_access = signed.tokens.access_token.clone();
    let fetched = account::call_signed_in(&client, &mut signed, &mut changed, |t| {
        license::fetch_licenses(&client, t)
    })
    .unwrap();
    assert!(changed && signed.tokens.access_token != old_access);
    let cached = license::merge_fetched(
        &license::load_cache(&home),
        &fetched,
        &signed.account.id,
        &ring,
        now_s(),
    );
    license::save_cache(&home, &cached).unwrap();
    let check = Entitlements::check(&license::load_cache(&home), &ring, now_s());
    assert!(check.refused.is_empty());
    assert!(check.entitlements.has("plugin:mock-paid"));
    assert!(check_entitlement(&paid_manifest, |e| check.entitlements.has(e)).is_ok());

    // The paid plugin runs (when python3 is installed).
    let config = PythonConfig {
        interpreter: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        sdk: None,
    };
    if interpreter_works(&config.interpreter) {
        let mut server = ferro_mcp::tools::Server::headless(Actor::user("user"));
        let r = plugins::run_command_blocking(
            &mut server.session,
            &config,
            &paid_plugin,
            "hello",
            &RunControl::new(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r["result"], "mock-paid ok", "{r}");
    } else {
        eprintln!("skipped running the plugin: no python3");
    }

    // Activation code (no sign-in needed) adds cloud.sync.
    let more = license::activate(&client, ACTIVATION_CODE, None).unwrap();
    assert!(Entitlements::check(&more, &ring, now_s())
        .entitlements
        .has("cloud.sync"));
    let e = license::activate(&client, "WRONG", None).unwrap_err();
    assert_eq!(e.code, codes::PROTOCOL);
    assert!(e.message.contains("not valid"), "{}", e.message);

    // Sign out: revoked tokens no longer work, and refresh fails too.
    account::revoke(&client, &signed).unwrap();
    account::clear(&home);
    assert!(account::load(&home).is_none());
    let e = license::fetch_licenses(&client, &signed.tokens.access_token).unwrap_err();
    assert_eq!(e.code, codes::UNAUTHORIZED);
    assert_eq!(
        account::refresh(&client, &signed).unwrap_err().code,
        codes::UNAUTHORIZED
    );
    let e = market::checkout(&client, &signed.tokens.access_token, PAID_PLUGIN).unwrap_err();
    assert_eq!(e.code, codes::UNAUTHORIZED);
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn offline_mode_makes_no_network_attempt() {
    // A live server at the env URL, but services.json says enabled: false.
    let mock = MockServer::start(MockOptions::default()).unwrap();
    let home = temp("offline");
    std::fs::write(
        home.join("services.json"),
        r#"{"base_url": "https://services.ferro.example", "enabled": false}"#,
    )
    .unwrap();
    let config = ServiceConfig::load_from(Some(&home), Some(&mock.url()));
    assert!(config.is_offline());
    let client = ServiceClient::new(config);
    let t = ferro_services::Secret::new("t");
    let errors = [
        account::start_device_flow(&client).map(|_| ()).unwrap_err(),
        license::fetch_licenses(&client, &t)
            .map(|_| ())
            .unwrap_err(),
        license::activate(&client, ACTIVATION_CODE, None)
            .map(|_| ())
            .unwrap_err(),
        market::search(&client, "", None).map(|_| ()).unwrap_err(),
        market::listing(&client, FREE_PLUGIN)
            .map(|_| ())
            .unwrap_err(),
        market::checkout(&client, &t, PAID_PLUGIN)
            .map(|_| ())
            .unwrap_err(),
    ];
    for e in errors {
        assert_eq!(e.code, codes::OFFLINE, "{e}");
    }
    assert_eq!(mock.requests(), 0, "no request reached the server");
    // And an unreachable URL with the offline flag fails offline at once.
    std::fs::write(
        home.join("services.json"),
        r#"{"base_url": "", "enabled": true}"#,
    )
    .unwrap();
    let c = ServiceClient::new(ServiceConfig::load_from(
        Some(&home),
        Some("http://127.0.0.1:1"),
    ));
    assert_eq!(
        market::search(&c, "", None).unwrap_err().code,
        codes::OFFLINE
    );
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn unreachable_server_is_unavailable() {
    // Port 1 on loopback: nothing listens.
    let c = ServiceClient::new(ServiceConfig::online("http://127.0.0.1:1", None).unwrap());
    assert_eq!(
        market::search(&c, "", None).unwrap_err().code,
        codes::UNAVAILABLE
    );
}

#[test]
fn tampered_and_expired_mock_tokens() {
    let mock = MockServer::start(MockOptions::default()).unwrap();
    let ring = keys(&mock);
    let t = mock.sign_license("mock-user", &["plugin:mock-paid"]);
    let ok = LicenseToken::verify(&t, &ring, now_s()).unwrap();
    assert_eq!(ok.state, TokenState::Valid);
    // Flip one character of the signature, then of the payload.
    let (p, s) = t.split_once('.').unwrap();
    let flip = |x: &str| {
        let mut c: Vec<char> = x.chars().collect();
        let i = c.len() / 2;
        c[i] = if c[i] == 'A' { 'B' } else { 'A' };
        c.into_iter().collect::<String>()
    };
    for bad in [format!("{p}.{}", flip(s)), format!("{}.{s}", flip(p))] {
        let e = LicenseToken::verify(&bad, &ring, now_s()).unwrap_err();
        assert_eq!(e.code, codes::LICENSE_INVALID);
    }
    // Development keys are not trusted for a remote server.
    let remote = ServiceConfig::online("https://services.ferro.example", None).unwrap();
    let remote_ring = KeyRing::builtin_with(&remote, Some(&mock.public_key_b64()));
    assert_eq!(
        LicenseToken::verify(&t, &remote_ring, now_s())
            .unwrap_err()
            .code,
        codes::LICENSE_INVALID
    );
    // Past exp: works in the grace period, refused after it.
    let exp = ok.payload.exp_s;
    let grace = LicenseToken::verify(&t, &ring, exp + 3 * 86_400).unwrap();
    assert_eq!(grace.state, TokenState::Grace { days_left: 11 });
    let check = Entitlements::check(std::slice::from_ref(&t), &ring, exp + 3 * 86_400);
    assert!(check.entitlements.has("plugin:mock-paid"));
    assert_eq!(check.warnings().len(), 1);
    let e = LicenseToken::verify(&t, &ring, exp + GRACE_S + 1).unwrap_err();
    assert_eq!(e.code, codes::LICENSE_EXPIRED);
    assert!(Entitlements::check(&[t], &ring, exp + GRACE_S + 1)
        .entitlements
        .is_empty());
}

#[test]
fn wrong_sha256_is_refused_and_nothing_is_installed() {
    let mock = MockServer::start(MockOptions::default()).unwrap();
    let home = temp("checksum");
    let client = ServiceClient::new(ServiceConfig::online(&mock.url(), Some(&home)).unwrap());
    let listing = market::listing(&client, FREE_PLUGIN).unwrap();
    mock.corrupt_download(FREE_PLUGIN);
    let dir = home.join("downloads");
    let e = market::download(&client, &listing, &dir).unwrap_err();
    assert_eq!(e.code, codes::MARKET_CHECKSUM);
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "the bad zip was deleted"
    );
    assert!(plugins::scan(&home.join("plugins")).is_empty());
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn plain_http_is_refused_except_for_localhost() {
    for url in [
        "http://services.ferro.example",
        "http://10.0.0.5:8080",
        "http://localhost.evil.example",
    ] {
        let e = ServiceConfig::online(url, None).unwrap_err();
        assert_eq!(e.code, codes::PROTOCOL, "{url}");
        // From services.json or the env URL: offline, with the reason.
        let c = ServiceConfig::load_from(None, Some(url));
        assert!(c.is_offline() && c.problem().is_some(), "{url}");
    }
    assert!(ServiceConfig::online("http://localhost:8787", None).is_ok());
    assert!(ServiceConfig::online("https://services.ferro.example", None).is_ok());
}

#[test]
fn cancelled_sign_in_stops_polling() {
    let mock = MockServer::start(MockOptions {
        approve_after_polls: 1000,
        ..MockOptions::default()
    })
    .unwrap();
    let client = ServiceClient::new(ServiceConfig::online(&mock.url(), None).unwrap());
    let start = account::start_device_flow(&client).unwrap();
    let cancel = AtomicBool::new(false);
    let mut n = 0;
    let mut sleep = |_| {
        n += 1;
        if n == 3 {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    };
    let r = account::wait_for_sign_in(&client, &start, &cancel, &mut sleep).unwrap();
    assert!(r.is_none());
    assert_eq!(mock.polls(), 2);
    // A device code that runs out of time ends the wait.
    let mut short = start.clone();
    short.expires_in_s = 2;
    let e = account::wait_for_sign_in(&client, &short, &AtomicBool::new(false), &mut no_sleep())
        .unwrap_err();
    assert_eq!(e.code, codes::PROTOCOL);
    assert!(e.message.contains("expired"));
}
