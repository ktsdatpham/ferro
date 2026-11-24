//! Account, licences and the services seam in the app (ADR 0010, M6-20.3 /
//! M6-20.4): File > Sign in… / Account… / Sign out, the sign-in window
//! (device flow: the user code, the browser opened on the verification
//! page), activation codes, the licence refresh at start and after a
//! purchase, and the [`Entitlements`] the plugin commands are checked
//! against.
//!
//! Every network call runs on a worker thread (`ferro_services` is
//! blocking) and reports back through a channel polled once per frame, so
//! the UI thread never waits. Tokens stay inside `ferro_services` types
//! ([`ferro_services::Secret`]): this module never reads their text, and
//! the session log only gets fixed phrases and failure codes.

use super::FerroApp;
use eframe::egui::{self, RichText};
use ferro_services::account::{self, DeviceStart, SignedIn};
use ferro_services::license::{self, LicenseCheck};
use ferro_services::market::{CatalogPage, Checkout, OrderStatus};
use ferro_services::text::now_s;
use ferro_services::{Entitlements, KeyRing, ServiceClient, ServiceConfig, ServiceError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

/// What a worker thread reports to the UI thread.
pub(super) enum ServiceEvent {
    /// The device flow started: show the code.
    DeviceStarted(DeviceStart),
    /// The user signed in on the web.
    SignedIn(SignedIn),
    /// The sign-in failed (or was cancelled: `None`).
    SignInEnded(Option<ServiceError>),
    /// Licence tokens fetched (signed in) or activated; the sign-in when
    /// its tokens were refreshed on the way.
    Licenses {
        tokens: Vec<String>,
        fetched_for: Option<String>,
        signed: Option<SignedIn>,
    },
    /// A licence call failed.
    LicensesFailed(ServiceError),
    /// Catalog results.
    Catalog(Result<CatalogPage, ServiceError>),
    /// A plugin zip was downloaded and checked (or not).
    Downloaded(Result<(PathBuf, String), ServiceError>),
    /// A checkout started (open the page) or failed; the sign-in when it
    /// was refreshed.
    Checkout(Result<(Checkout, String), ServiceError>, Option<SignedIn>),
    /// An order changed (or polling failed).
    Order(Result<OrderStatus, ServiceError>),
}

/// The sign-in in progress.
struct SignInFlow {
    start: Option<DeviceStart>,
    cancel: Arc<AtomicBool>,
}

/// Account and licence state of the app.
pub(super) struct AccountState {
    pub(super) config: ServiceConfig,
    pub(super) client: ServiceClient,
    keys: KeyRing,
    pub(super) signed_in: Option<SignedIn>,
    tokens: Vec<String>,
    pub(super) entitlements: Entitlements,
    warnings: Vec<String>,
    flow: Option<SignInFlow>,
    error: Option<String>,
    /// The account window is open.
    pub show_window: bool,
    activation: String,
    busy_licenses: bool,
    pub(super) tx: Sender<ServiceEvent>,
    rx: Receiver<ServiceEvent>,
    ctx: Option<egui::Context>,
}

/// A message for the status line: text, and whether it is an error.
pub(super) type Note = (String, bool);

impl AccountState {
    /// Load the configuration, the stored sign-in and the cached licence
    /// tokens of `home` (the Ferro folder), and check the tokens offline.
    pub fn load_from(config: ServiceConfig) -> AccountState {
        let keys = KeyRing::builtin(&config);
        let home = config.home().map(Path::to_path_buf);
        let signed_in = home.as_deref().and_then(account::load);
        let tokens = home.as_deref().map(license::load_cache).unwrap_or_default();
        let client = ServiceClient::new(config.clone());
        let (tx, rx) = channel();
        let mut s = AccountState {
            config,
            client,
            keys,
            signed_in,
            tokens,
            entitlements: Entitlements::none(),
            warnings: Vec::new(),
            flow: None,
            error: None,
            show_window: false,
            activation: String::new(),
            busy_licenses: false,
            tx,
            rx,
            ctx: None,
        };
        s.recheck();
        s
    }

    fn home(&self) -> Option<PathBuf> {
        self.config.home().map(Path::to_path_buf)
    }

    /// Check the cached tokens again (at start and after each refresh or
    /// activation).
    fn recheck(&mut self) -> LicenseCheck {
        let check = license::Entitlements::check(&self.tokens, &self.keys, now_s());
        self.entitlements = check.entitlements.clone();
        self.warnings = check.warnings();
        check
    }

    /// The signed-in account's name, if signed in.
    pub fn account_name(&self) -> Option<&str> {
        self.signed_in.as_ref().map(|s| s.account.name.as_str())
    }

    fn spawn(&self, f: impl FnOnce(&ServiceClient, &Sender<ServiceEvent>) + Send + 'static) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            f(&client, &tx);
            if let Some(c) = ctx {
                c.request_repaint();
            }
        });
    }

    /// Run `f` with the sign-in on a worker thread (refreshing the access
    /// token when needed); `done` turns the result and the refreshed
    /// sign-in (if it changed) into an event.
    pub(super) fn spawn_signed_in<T: Send + 'static>(
        &self,
        f: impl Fn(&ServiceClient, &ferro_services::Secret) -> Result<T, ServiceError> + Send + 'static,
        done: impl FnOnce(Result<T, ServiceError>, Option<SignedIn>) -> ServiceEvent + Send + 'static,
    ) -> bool {
        let Some(mut signed) = self.signed_in.clone() else {
            return false;
        };
        self.spawn(move |client, tx| {
            let mut changed = false;
            let r = account::call_signed_in(client, &mut signed, &mut changed, |t| f(client, t));
            let _ = tx.send(done(r, changed.then_some(signed)));
        });
        true
    }

    /// Fetch the signed-in account's licences in the background (at start
    /// and after a purchase). Does nothing offline or signed out.
    pub fn refresh_licenses(&mut self) {
        if self.config.is_offline() || self.busy_licenses {
            return;
        }
        let id = match &self.signed_in {
            Some(s) => s.account.id.clone(),
            None => return,
        };
        self.busy_licenses =
            self.spawn_signed_in(license::fetch_licenses, move |r, signed| match r {
                Ok(tokens) => ServiceEvent::Licenses {
                    tokens,
                    fetched_for: Some(id),
                    signed,
                },
                Err(e) => ServiceEvent::LicensesFailed(e),
            });
    }

    /// Start a sign-in: ask for a device code on a worker thread, then
    /// poll until the user has signed in on the web.
    pub fn start_sign_in(&mut self) -> Option<Note> {
        if self.config.is_offline() {
            return Some((ServiceError::offline().to_string(), true));
        }
        if let Some(f) = self.flow.take() {
            f.cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.flow = Some(SignInFlow {
            start: None,
            cancel: cancel.clone(),
        });
        self.error = None;
        self.show_window = true;
        crate::report::record("account sign-in started");
        self.spawn(move |client, tx| {
            let start = match account::start_device_flow(client) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(ServiceEvent::SignInEnded(Some(e)));
                    return;
                }
            };
            let _ = tx.send(ServiceEvent::DeviceStarted(start.clone()));
            let r = account::wait_for_sign_in(client, &start, &cancel, &mut |d| {
                // Sleep in short steps so Cancel answers quickly.
                let mut left = d;
                while !left.is_zero() && !cancel.load(Ordering::Relaxed) {
                    let step = left.min(Duration::from_millis(200));
                    std::thread::sleep(step);
                    left -= step;
                }
            });
            let _ = tx.send(match r {
                Ok(Some(s)) => ServiceEvent::SignedIn(s),
                Ok(None) => ServiceEvent::SignInEnded(None),
                Err(e) => ServiceEvent::SignInEnded(Some(e)),
            });
        });
        None
    }

    /// Cancel the sign-in in progress.
    pub fn cancel_sign_in(&mut self) {
        if let Some(f) = self.flow.take() {
            f.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Sign out: revoke on the server in the background (best effort) and
    /// forget the tokens here at once. Licence tokens already cached keep
    /// working until they expire (they are signed and offline).
    pub fn sign_out(&mut self) -> Note {
        let Some(signed) = self.signed_in.take() else {
            return ("Not signed in".into(), false);
        };
        if let Some(h) = self.home() {
            account::clear(&h);
        }
        crate::report::record("account signed out");
        self.spawn(move |client, _| {
            let _ = account::revoke(client, &signed);
        });
        ("Signed out".into(), false)
    }

    /// Redeem the typed activation code in the background.
    fn activate(&mut self) {
        let code = self.activation.trim().to_string();
        let access = self
            .signed_in
            .as_ref()
            .map(|s| s.tokens.access_token.clone());
        self.busy_licenses = true;
        self.spawn(move |client, tx| {
            let _ = tx.send(match license::activate(client, &code, access.as_ref()) {
                Ok(tokens) => ServiceEvent::Licenses {
                    tokens,
                    fetched_for: None,
                    signed: None,
                },
                Err(e) => ServiceEvent::LicensesFailed(e),
            });
        });
    }

    fn store_signed(&mut self, s: SignedIn) -> Option<Note> {
        let note = self
            .home()
            .and_then(|h| account::save(&h, &s).err())
            .map(|e| {
                (
                    format!("Signed in, but the sign-in could not be saved: {e}"),
                    true,
                )
            });
        self.signed_in = Some(s);
        note
    }

    /// Apply an account / licence event; returns what to tell the user.
    /// Marketplace events are returned unchanged for the marketplace.
    pub(super) fn handle(&mut self, ev: ServiceEvent) -> Result<Vec<Note>, Box<ServiceEvent>> {
        let mut notes = Vec::new();
        match ev {
            ServiceEvent::DeviceStarted(start) => {
                if let Some(f) = &mut self.flow {
                    let opened = crate::report::open_url(&start.verification_uri).is_ok();
                    if !opened {
                        self.error = Some(
                            "Could not open the browser: open the page below yourself.".into(),
                        );
                    }
                    f.start = Some(start);
                }
            }
            ServiceEvent::SignedIn(s) => {
                self.flow = None;
                crate::report::record("account signed in");
                notes.push((format!("Signed in as {}", s.account.name), false));
                notes.extend(self.store_signed(s));
                self.show_window = false;
                self.refresh_licenses();
            }
            ServiceEvent::SignInEnded(e) => {
                self.flow = None;
                if let Some(e) = e {
                    crate::report::record(format!("account sign-in failed {}", e.code));
                    self.error = Some(e.to_string());
                    notes.push((e.to_string(), true));
                }
            }
            ServiceEvent::Licenses {
                tokens,
                fetched_for,
                signed,
            } => {
                self.busy_licenses = false;
                if let Some(s) = signed {
                    notes.extend(self.store_signed(s));
                }
                self.tokens = match &fetched_for {
                    Some(id) => {
                        license::merge_fetched(&self.tokens, &tokens, id, &self.keys, now_s())
                    }
                    None => {
                        self.activation.clear();
                        let mut t = self.tokens.clone();
                        t.extend(tokens.iter().cloned());
                        t
                    }
                };
                if let Some(h) = self.home() {
                    if let Err(e) = license::save_cache(&h, &self.tokens) {
                        notes.push((format!("Licences could not be saved: {e}"), true));
                    }
                }
                let check = self.recheck();
                crate::report::record(format!(
                    "licences {} valid {} refused",
                    check.tokens.len(),
                    check.refused.len()
                ));
                if fetched_for.is_none() {
                    notes.push(("Activation code accepted".into(), false));
                }
                if let Some(w) = self.warnings.first() {
                    notes.push((w.clone(), true));
                }
            }
            ServiceEvent::LicensesFailed(e) => {
                self.busy_licenses = false;
                crate::report::record(format!("licences failed {}", e.code));
                if e.code == ferro_services::codes::UNAUTHORIZED {
                    // The sign-in is gone on the server: forget it here.
                    self.signed_in = None;
                    if let Some(h) = self.home() {
                        account::clear(&h);
                    }
                }
                self.error = Some(e.to_string());
                notes.push((e.to_string(), true));
            }
            other => return Err(Box::new(other)),
        }
        Ok(notes)
    }
}

impl FerroApp {
    /// Once per frame: apply what the service workers reported.
    pub(super) fn poll_services(&mut self, ctx: &egui::Context) {
        if self.account.ctx.is_none() {
            self.account.ctx = Some(ctx.clone());
            // At start: refresh licences in the background.
            self.account.refresh_licenses();
        }
        while let Ok(ev) = self.account.rx.try_recv() {
            match self.account.handle(ev) {
                Ok(notes) => {
                    for (t, e) in notes {
                        self.say(t, e);
                    }
                }
                Err(ev) => self.market_event(*ev),
            }
        }
        if self.account.flow.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// File menu entries: Sign in… / the account name, Sign out.
    pub(super) fn account_menu(&mut self, ui: &mut egui::Ui) {
        match self.account.account_name().map(str::to_string) {
            Some(name) => {
                if ui.button(format!("Account: {name}…")).clicked() {
                    self.account.show_window = true;
                    ui.close();
                }
                if ui.button("Sign out").clicked() {
                    let (t, e) = self.account.sign_out();
                    self.say(t, e);
                    ui.close();
                }
            }
            None => {
                if ui.button("Sign in…").clicked() {
                    if let Some((t, e)) = self.account.start_sign_in() {
                        self.say(t, e);
                    }
                    self.account.show_window = true;
                    ui.close();
                }
            }
        }
    }

    /// The account window: sign-in code while signing in; else the
    /// account, its licences, an activation code field and Sign out.
    pub(super) fn account_window(&mut self, ctx: &egui::Context) {
        if !self.account.show_window {
            return;
        }
        let mut open = true;
        let mut act: Option<&'static str> = None;
        let a = &mut self.account;
        egui::Window::new("Ferro account")
            .open(&mut open)
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                if a.config.is_offline() {
                    ui.label("Services are off: Ferro works fully without an account.");
                    if let Some(p) = a.config.problem() {
                        ui.label(RichText::new(p).weak());
                    }
                    ui.label(RichText::new("Turn them on in services.json in the Ferro folder.").weak());
                    return;
                }
                if let Some(flow) = &a.flow {
                    match &flow.start {
                        None => {
                            ui.label("Contacting the account service…");
                        }
                        Some(start) => {
                            ui.label("Sign in on the web page that opened in your browser, and type this code there:");
                            ui.add_space(4.0);
                            ui.label(RichText::new(&start.user_code).size(28.0).strong().monospace());
                            ui.add_space(4.0);
                            ui.label(RichText::new(&start.verification_uri).weak());
                            ui.label(RichText::new("Ferro never asks for your password.").weak());
                            ui.horizontal(|ui| {
                                if ui.button("Open the page again").clicked() {
                                    act = Some("open_page");
                                }
                                if ui.button("Copy code").clicked() {
                                    ui.ctx().copy_text(start.user_code.clone());
                                }
                            });
                            ui.label(RichText::new("Waiting for you to sign in…").italics());
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        act = Some("cancel");
                    }
                } else if let Some(s) = &a.signed_in {
                    ui.label(RichText::new(&s.account.name).strong());
                    if !s.account.email.is_empty() {
                        ui.label(RichText::new(&s.account.email).weak());
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!a.busy_licenses, egui::Button::new("Refresh licences")).clicked() {
                            act = Some("refresh");
                        }
                        if ui.button("Sign out").clicked() {
                            act = Some("sign_out");
                        }
                    });
                } else {
                    ui.label("Sign in to buy plugins in the Marketplace and use Ferro services.");
                    ui.label(RichText::new("You sign in on the web: Ferro never asks for your password.").weak());
                    if ui.button("Sign in…").clicked() {
                        act = Some("sign_in");
                    }
                }
                ui.separator();
                ui.label(RichText::new("Licences").strong());
                let ids: Vec<&str> = a.entitlements.ids().collect();
                if ids.is_empty() {
                    ui.label(RichText::new("None yet.").weak());
                } else {
                    for id in ids {
                        ui.label(format!("  {id}"));
                    }
                }
                for w in &a.warnings {
                    ui.label(RichText::new(w).color(egui::Color32::from_rgb(200, 120, 40)));
                }
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut a.activation)
                            .hint_text("Activation code")
                            .desired_width(200.0),
                    );
                    let ok = !a.activation.trim().is_empty() && !a.busy_licenses;
                    if ui.add_enabled(ok, egui::Button::new("Activate")).clicked() {
                        act = Some("activate");
                    }
                });
                if let Some(e) = &a.error {
                    ui.colored_label(egui::Color32::from_rgb(200, 60, 40), e);
                }
            });
        self.account.show_window = open;
        if !open {
            self.account.cancel_sign_in();
        }
        match act {
            Some("open_page") => {
                let uri = self
                    .account
                    .flow
                    .as_ref()
                    .and_then(|f| f.start.as_ref())
                    .map(|s| s.verification_uri.clone());
                if let Some(u) = uri {
                    if let Err(e) = crate::report::open_url(&u) {
                        self.say(format!("Cannot open the browser: {e}"), true);
                    }
                }
            }
            Some("cancel") => self.account.cancel_sign_in(),
            Some("refresh") => self.account.refresh_licenses(),
            Some("sign_out") => {
                let (t, e) = self.account.sign_out();
                self.say(t, e);
            }
            Some("sign_in") => {
                if let Some((t, e)) = self.account.start_sign_in() {
                    self.say(t, e);
                }
            }
            Some("activate") => self.account.activate(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_services::account::{Account, AccountTokens};
    use ferro_services::Secret;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ferro-app-account-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn tokens_never_reach_the_session_log_or_the_status_line() {
        let home = temp();
        let config = ServiceConfig::online("http://127.0.0.1:1", Some(&home)).unwrap();
        let mut s = AccountState::load_from(config);
        assert!(s.signed_in.is_none());
        let signed = SignedIn {
            account: Account {
                id: "u1".into(),
                name: "An".into(),
                email: "an@example.com".into(),
            },
            tokens: AccountTokens {
                access_token: Secret::new("ACCESS-TOKEN-SECRET-1"),
                refresh_token: Secret::new("REFRESH-TOKEN-SECRET-2"),
                expires_at_s: now_s() + 3600,
            },
        };
        let mut notes = s.handle(ServiceEvent::SignedIn(signed)).ok().unwrap();
        notes.extend(
            s.handle(ServiceEvent::Licenses {
                tokens: vec!["LICENSE-TOKEN-3".into()],
                fetched_for: Some("u1".into()),
                signed: None,
            })
            .ok()
            .unwrap(),
        );
        notes.extend(
            s.handle(ServiceEvent::LicensesFailed(ServiceError::new(
                ferro_services::codes::UNAUTHORIZED,
                "expired",
            )))
            .ok()
            .unwrap(),
        );
        let report = crate::report::report_text("test");
        let said: String = notes
            .iter()
            .map(|(t, _)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for secret in ["ACCESS-TOKEN-SECRET-1", "REFRESH-TOKEN-SECRET-2"] {
            assert!(!report.contains(secret), "report leaks a token");
            assert!(!said.contains(secret), "status line leaks a token");
        }
        assert!(report.contains("account signed in"));
        // The unauthorized answer signed the user out here.
        assert!(s.signed_in.is_none());
        assert!(account::load(&home).is_none());
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn the_app_never_reads_token_text() {
        // Tokens are read (Secret::expose) only inside ferro-services: no
        // app source may call it, so no token can reach a log, a problem
        // report, an MCP answer or a plugin's environment from here.
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        let needle = [".expose", "()"].concat();
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    let text = std::fs::read_to_string(&p).unwrap();
                    assert!(!text.contains(&needle), "{} reads a token", p.display());
                }
            }
        }
    }

    #[test]
    fn offline_sign_in_says_offline_and_starts_nothing() {
        let mut s = AccountState::load_from(ServiceConfig::offline(None));
        let (text, error) = s.start_sign_in().unwrap();
        assert!(error && text.starts_with(ferro_services::codes::OFFLINE));
        assert!(s.flow.is_none());
        s.refresh_licenses();
        assert!(!s.busy_licenses);
    }
}
