//! The Marketplace window (ADR 0010, M6-20.5): search the catalog, read a
//! listing, Install (download, SHA-256 check, then the plugin install
//! screen of ADR 0009) and Buy (checkout page in the browser, order
//! polling, licence refresh when paid).
//!
//! Network calls run on worker threads through [`super::account`]'s
//! channel. Everything shown comes from `ferro_services::market` types,
//! which sanitise the server's text and refuse unsafe ids and URLs.

use super::account::ServiceEvent;
use super::FerroApp;
use eframe::egui::{self, RichText};
use ferro_services::market::{self, CatalogPage, Listing, OrderStatus};
use ferro_services::ServiceError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often a pending order is polled.
const ORDER_POLL: Duration = Duration::from_secs(3);
/// How long an order is polled before giving up.
const ORDER_WAIT: Duration = Duration::from_secs(30 * 60);

/// A purchase waiting for its payment.
struct PendingOrder {
    plugin_id: String,
    cancel: Arc<AtomicBool>,
}

/// Marketplace state of the app.
#[derive(Default)]
pub(super) struct MarketState {
    /// The Marketplace window is open.
    pub show_window: bool,
    query: String,
    results: Vec<Listing>,
    selected: Option<String>,
    busy: Option<&'static str>,
    message: Option<(String, bool)>,
    order: Option<PendingOrder>,
    searched: bool,
}

impl FerroApp {
    /// True when the plugin is paid and its entitlement is not owned: its
    /// commands show "Buy" and refuse to run (`ferro.license.missing`).
    pub(super) fn plugin_locked(&self, plugin_id: &str) -> bool {
        self.plugins
            .installed(plugin_id)
            .and_then(|m| m.required_entitlement())
            .is_some_and(|e| !self.account.entitlements.has(&e))
    }

    /// Open the Marketplace on one listing (from a refused paid command).
    pub(super) fn open_listing(&mut self, plugin_id: &str) {
        self.market.show_window = true;
        self.market.selected = Some(plugin_id.to_string());
        if self.market.results.iter().any(|l| l.id == plugin_id) {
            return;
        }
        let id = plugin_id.to_string();
        self.market.busy = Some("Loading…");
        let tx = self.account.tx.clone();
        let client = self.account.client.clone();
        std::thread::spawn(move || {
            let r = market::listing(&client, &id).map(|l| CatalogPage {
                plugins: vec![l],
                skipped: 0,
                next: None,
            });
            let _ = tx.send(ServiceEvent::Catalog(r));
        });
    }

    fn market_search(&mut self) {
        let q = self.market.query.clone();
        self.market.busy = Some("Searching…");
        self.market.searched = true;
        let tx = self.account.tx.clone();
        let client = self.account.client.clone();
        std::thread::spawn(move || {
            let _ = tx.send(ServiceEvent::Catalog(market::search(&client, &q, None)));
        });
    }

    fn market_install(&mut self, listing: Listing) {
        self.market.busy = Some("Downloading…");
        let tx = self.account.tx.clone();
        let client = self.account.client.clone();
        std::thread::spawn(move || {
            let dir = std::env::temp_dir()
                .join(format!("ferro-market-{}", uuid::Uuid::new_v4().simple()));
            let r = market::download(&client, &listing, &dir).map(|p| (p, listing.name.clone()));
            let _ = tx.send(ServiceEvent::Downloaded(r));
        });
    }

    fn market_buy(&mut self, listing: &Listing) {
        if self.account.signed_in.is_none() {
            self.market.message = Some(("Sign in to buy plugins.".into(), true));
            if let Some((t, e)) = self.account.start_sign_in() {
                self.say(t, e);
            }
            return;
        }
        let id = listing.id.clone();
        self.market.busy = Some("Starting the checkout…");
        let pid = id.clone();
        self.account.spawn_signed_in(
            move |c, t| market::checkout(c, t, &id),
            move |r, signed| ServiceEvent::Checkout(r.map(|co| (co, pid)), signed),
        );
    }

    fn poll_order(&mut self, order_id: String, plugin_id: String) {
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(old) = self.market.order.replace(PendingOrder {
            plugin_id,
            cancel: cancel.clone(),
        }) {
            old.cancel.store(true, Ordering::Relaxed);
        }
        let Some(mut signed) = self.account.signed_in.clone() else {
            return;
        };
        let tx = self.account.tx.clone();
        let client = self.account.client.clone();
        std::thread::spawn(move || {
            let begun = Instant::now();
            loop {
                let mut waited = Duration::ZERO;
                while waited < ORDER_POLL {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                    waited += Duration::from_millis(200);
                }
                let mut changed = false;
                let r = ferro_services::account::call_signed_in(
                    &client,
                    &mut signed,
                    &mut changed,
                    |t| market::order_status(&client, t, &order_id),
                );
                match r {
                    Ok(OrderStatus::Pending) if begun.elapsed() < ORDER_WAIT => {}
                    Ok(OrderStatus::Pending) => {
                        let _ = tx.send(ServiceEvent::Order(Err(ServiceError::protocol(
                            "still not paid after 30 minutes: check your orders on the web",
                        ))));
                        return;
                    }
                    r => {
                        let _ = tx.send(ServiceEvent::Order(r));
                        return;
                    }
                }
            }
        });
    }

    /// A marketplace event from a worker thread.
    pub(super) fn market_event(&mut self, ev: ServiceEvent) {
        self.market.busy = None;
        match ev {
            ServiceEvent::Catalog(Ok(page)) => {
                let skipped = page.skipped;
                self.market.results = page.plugins;
                self.market.message = (skipped > 0).then(|| {
                    (
                        format!("{skipped} listing(s) could not be read and are hidden"),
                        true,
                    )
                });
            }
            ServiceEvent::Downloaded(Ok((path, name))) => {
                crate::report::record("market plugin downloaded");
                self.market.message =
                    Some((format!("Downloaded {name}: confirm the install"), false));
                self.stage_plugin(&path);
                if let Some(dir) = path.parent() {
                    let _ = std::fs::remove_dir_all(dir);
                }
            }
            ServiceEvent::Checkout(Ok((co, plugin_id)), signed) => {
                if let Some(s) = signed {
                    self.account.signed_in = Some(s);
                }
                crate::report::record("market checkout started");
                let opened = crate::report::open_url(&co.checkout_url).is_ok();
                self.market.message = Some((
                    if opened {
                        "Pay in your browser; Ferro waits for the payment…".into()
                    } else {
                        "Could not open the browser for the payment".into()
                    },
                    !opened,
                ));
                self.poll_order(co.order_id, plugin_id);
            }
            ServiceEvent::Order(Ok(status)) => {
                let plugin = self
                    .market
                    .order
                    .take()
                    .map(|o| o.plugin_id)
                    .unwrap_or_default();
                crate::report::record(format!("market order {}", status.as_str()));
                match status {
                    OrderStatus::Paid => {
                        self.market.message = Some((
                            format!("Paid: {plugin} is yours. Refreshing licences…"),
                            false,
                        ));
                        self.account.refresh_licenses();
                    }
                    OrderStatus::Failed => {
                        self.market.message = Some(("The payment failed".into(), true))
                    }
                    OrderStatus::Cancelled => {
                        self.market.message = Some(("The payment was cancelled".into(), true))
                    }
                    OrderStatus::Pending => {}
                }
            }
            ServiceEvent::Order(Err(e)) => {
                // The order poller stopped.
                self.market.order = None;
                crate::report::record(format!("market failed {}", e.code));
                self.market.message = Some((e.to_string(), true));
                self.say(e.to_string(), true);
            }
            ServiceEvent::Catalog(Err(e))
            | ServiceEvent::Downloaded(Err(e))
            | ServiceEvent::Checkout(Err(e), _) => {
                crate::report::record(format!("market failed {}", e.code));
                self.market.message = Some((e.to_string(), true));
                self.say(e.to_string(), true);
            }
            _ => {}
        }
    }

    /// The Marketplace window.
    pub(super) fn marketplace_window(&mut self, ctx: &egui::Context) {
        if !self.market.show_window {
            return;
        }
        if !self.market.searched && !self.account.config.is_offline() {
            self.market_search();
        }
        let mut open = true;
        let mut search = false;
        let mut install: Option<Listing> = None;
        let mut buy: Option<Listing> = None;
        let mut cancel_order = false;
        let mut select: Option<Option<String>> = None;
        egui::Window::new("Marketplace")
            .open(&mut open)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                if self.account.config.is_offline() {
                    ui.label("Services are off: the Marketplace needs them (services.json).");
                    ui.label(
                        RichText::new(
                            "Plugins from a file or a folder install without it (Plugins menu).",
                        )
                        .weak(),
                    );
                    return;
                }
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.market.query)
                            .hint_text("Search plugins")
                            .desired_width(300.0),
                    );
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Search").clicked() || enter {
                        search = true;
                    }
                    if let Some(b) = self.market.busy {
                        ui.spinner();
                        ui.label(RichText::new(b).weak());
                    }
                });
                if let Some((m, err)) = &self.market.message {
                    if *err {
                        ui.colored_label(egui::Color32::from_rgb(200, 60, 40), m);
                    } else {
                        ui.label(m);
                    }
                }
                if self.market.results.is_empty() && self.market.busy.is_none() {
                    ui.label(RichText::new("No plugins found.").weak());
                }
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for l in &self.market.results {
                            ui.separator();
                            let installed =
                                self.plugins.installed(&l.id).map(|m| m.version.clone());
                            let owned =
                                l.is_free() || self.account.entitlements.has(&l.entitlement);
                            let waiting = self
                                .market
                                .order
                                .as_ref()
                                .is_some_and(|o| o.plugin_id == l.id);
                            ui.horizontal(|ui| {
                                let is_sel = self.market.selected.as_deref() == Some(l.id.as_str());
                                if ui
                                    .selectable_label(is_sel, RichText::new(&l.name).strong())
                                    .clicked()
                                {
                                    select = Some(if is_sel { None } else { Some(l.id.clone()) });
                                }
                                ui.label(RichText::new(l.price.label()).weak());
                            });
                            let by = if l.publisher.verified {
                                format!("by {} (verified)", l.publisher.name)
                            } else {
                                format!("by {}", l.publisher.name)
                            };
                            ui.label(RichText::new(by).weak());
                            if !l.summary.is_empty() {
                                ui.label(&l.summary);
                            }
                            if self.market.selected.as_deref() == Some(l.id.as_str()) {
                                ui.label(format!("Version {}", l.version));
                                if let Some(r) = l.rating {
                                    ui.label(format!("Rating {r:.1} / 5, {} installs", l.installs));
                                }
                                if l.size > 0 {
                                    ui.label(
                                        RichText::new(format!("{} KB", l.size.div_ceil(1024)))
                                            .weak(),
                                    );
                                }
                            }
                            ui.horizontal(|ui| {
                                match &installed {
                                    Some(v) => {
                                        ui.label(
                                            RichText::new(format!("Installed ({v})")).italics(),
                                        );
                                        if owned
                                            && *v != l.version
                                            && ui
                                                .small_button(format!("Update to {}", l.version))
                                                .clicked()
                                        {
                                            install = Some(l.clone());
                                        }
                                    }
                                    None if owned && ui.button("Install").clicked() => {
                                        install = Some(l.clone());
                                    }
                                    None => {}
                                }
                                if !owned {
                                    if waiting {
                                        ui.label(
                                            RichText::new("Waiting for the payment…").italics(),
                                        );
                                        if ui.small_button("Stop waiting").clicked() {
                                            cancel_order = true;
                                        }
                                    } else if ui
                                        .button(format!("Buy  {}", l.price.label()))
                                        .clicked()
                                    {
                                        buy = Some(l.clone());
                                    }
                                }
                            });
                        }
                    });
                ui.separator();
                ui.label(
                    RichText::new("Plugins run with your rights, like any program you install.")
                        .weak(),
                );
            });
        self.market.show_window = open;
        if let Some(s) = select {
            self.market.selected = s;
        }
        if search {
            self.market_search();
        }
        if let Some(l) = install {
            self.market_install(l);
        }
        if let Some(l) = buy {
            self.market_buy(&l);
        }
        if cancel_order {
            if let Some(o) = self.market.order.take() {
                o.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
}
