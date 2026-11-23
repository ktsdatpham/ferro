//! The *Project settings…* window (M5-24.2): display units and decimals per
//! quantity, site location and currency.
//!
//! The window never changes the document itself: every edit becomes a
//! `set_project_units`, `set_site_location` or `set_project_currency`
//! command that the app runs (one undo step each).

use crate::units;
use eframe::egui::{self, Color32, RichText};
use ferro_core::math::{Angle, QuantitySpec};
use ferro_core::model::project_settings::ProjectSettings;
use ferro_core::ops::project_commands::{SetProjectCurrency, SetProjectUnits, SetSiteLocation};
use ferro_core::{AnyCommand, Document, Length};

/// Quantities listed in the window, in this order: the geometric ones first,
/// then those used by material data (thermal, mass, carbon).
pub const LISTED: [QuantitySpec; 12] = [
    QuantitySpec::Length,
    QuantitySpec::Area,
    QuantitySpec::Volume,
    QuantitySpec::Angle,
    QuantitySpec::Mass,
    QuantitySpec::Density,
    QuantitySpec::ThermalConductivity,
    QuantitySpec::ThermalResistance,
    QuantitySpec::UValue,
    QuantitySpec::HeatCapacity,
    QuantitySpec::Temperature,
    QuantitySpec::CarbonMass,
];

/// Text being edited in the window; reloaded from the document whenever the
/// document's settings change (apply, undo, an agent).
#[derive(Debug, Clone, Default)]
pub struct ProjectSettingsDraft {
    loaded: Option<ProjectSettings>,
    place: String,
    latitude: String,
    longitude: String,
    elevation: String,
    utc_offset: String,
    weather_file: String,
    currency: String,
    error: Option<String>,
}

impl ProjectSettingsDraft {
    fn reload(&mut self, doc: &Document) {
        let s = &doc.project().settings;
        let u = &s.units;
        match &s.location {
            Some(l) => {
                self.place = l.place.clone();
                self.latitude = trim_deg(l.latitude.deg());
                self.longitude = trim_deg(l.longitude.deg());
                self.elevation = units::length(u, l.elevation.mm());
                self.utc_offset = format_utc_offset(l.utc_offset_minutes);
                self.weather_file = l.weather_file.clone().unwrap_or_default();
            }
            None => {
                self.place.clear();
                self.latitude.clear();
                self.longitude.clear();
                self.elevation.clear();
                self.utc_offset.clear();
                self.weather_file.clear();
            }
        }
        self.currency = s.currency.clone().unwrap_or_default();
        self.loaded = Some(s.clone());
    }

    /// Draw the window contents. Returns the commands to run (empty when
    /// nothing changed). Invalid text is reported in the window, not run.
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document) -> Vec<AnyCommand> {
        if self.loaded.as_ref() != Some(&doc.project().settings) {
            self.reload(doc);
        }
        let settings = doc.project().settings.clone();
        let mut out: Vec<AnyCommand> = Vec::new();

        ui.heading("Units");
        ui.label(
            RichText::new("How values are shown and how bare typed numbers are read.")
                .small()
                .weak(),
        );
        egui::Grid::new("project_units")
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                for spec in LISTED {
                    let choice = settings.units.units.get(&spec);
                    let current = settings.units.display_unit(spec).symbol;
                    let decimals = choice.and_then(|c| c.decimals);
                    ui.label(spec_label(spec));
                    let mut unit = current;
                    egui::ComboBox::from_id_salt(("unit", spec.id()))
                        .selected_text(unit_label(unit))
                        .show_ui(ui, |ui| {
                            for d in spec.display_units() {
                                ui.selectable_value(&mut unit, d.symbol, unit_label(d.symbol));
                            }
                        });
                    let mut dec = decimals;
                    egui::ComboBox::from_id_salt(("dec", spec.id()))
                        .width(60.0)
                        .selected_text(match dec {
                            Some(d) => format!("{d} dp"),
                            None => "auto".into(),
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut dec, None, "auto");
                            for d in 0..=6u8 {
                                ui.selectable_value(&mut dec, Some(d), format!("{d} dp"));
                            }
                        });
                    ui.end_row();
                    if unit != current || dec != decimals {
                        out.push(
                            SetProjectUnits {
                                spec,
                                unit: Some(unit.to_string()),
                                decimals: dec,
                            }
                            .into(),
                        );
                    }
                }
            });
        ui.label(
            RichText::new(format!(
                "e.g. {} · {} · {}",
                units::length(&settings.units, 3300.0),
                units::area(&settings.units, 12e6),
                units::angle(&settings.units, std::f64::consts::FRAC_PI_4),
            ))
            .weak(),
        );

        ui.separator();
        ui.heading("Site location");
        egui::Grid::new("site_location")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Place");
                ui.text_edit_singleline(&mut self.place);
                ui.end_row();
                ui.label("Latitude (°, N +)");
                ui.text_edit_singleline(&mut self.latitude);
                ui.end_row();
                ui.label("Longitude (°, E +)");
                ui.text_edit_singleline(&mut self.longitude);
                ui.end_row();
                ui.label("Elevation");
                ui.text_edit_singleline(&mut self.elevation)
                    .on_hover_text("Height of the project origin above sea level");
                ui.end_row();
                ui.label("UTC offset");
                ui.add(egui::TextEdit::singleline(&mut self.utc_offset).hint_text("+07:00"));
                ui.end_row();
                ui.label("Weather file");
                ui.add(
                    egui::TextEdit::singleline(&mut self.weather_file).hint_text("path to .epw"),
                );
                ui.end_row();
            });
        ui.horizontal(|ui| {
            if ui.button("Apply location").clicked() {
                match self.location_command(&settings) {
                    Ok(c) => {
                        self.error = None;
                        out.push(c.into());
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            if settings.location.is_some() && ui.button("Clear").clicked() {
                out.push(
                    SetSiteLocation {
                        latitude: None,
                        longitude: None,
                        elevation: None,
                        utc_offset_minutes: None,
                        place: None,
                        weather_file: None,
                        clear: true,
                    }
                    .into(),
                );
            }
        });

        ui.separator();
        ui.heading("Currency");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.currency)
                    .desired_width(60.0)
                    .hint_text("VND"),
            );
            if ui.button("Apply").clicked() {
                let c = self.currency.trim();
                let currency = (!c.is_empty()).then(|| c.to_string());
                if currency != settings.currency {
                    out.push(SetProjectCurrency { currency }.into());
                }
            }
        });
        ui.label(
            RichText::new("ISO 4217 code. Costs are not converted; other currencies are flagged.")
                .small()
                .weak(),
        );
        if let Some(e) = &self.error {
            ui.colored_label(Color32::from_rgb(200, 40, 40), e);
        }
        out
    }

    /// The `set_site_location` command for the typed text. Fails with a
    /// message naming the field that could not be read.
    fn location_command(&self, s: &ProjectSettings) -> Result<SetSiteLocation, String> {
        let deg = |t: &str, what: &str| -> Result<Angle, String> {
            t.trim()
                .trim_end_matches('°')
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .map(Angle::from_deg)
                .ok_or_else(|| format!("{what}: type decimal degrees, e.g. 16.05"))
        };
        let elevation = match self.elevation.trim() {
            "" => Length::ZERO,
            t => units::parse_length(&s.units, t)
                .map(Length::from_mm)
                .ok_or_else(|| format!("elevation: cannot read {t:?}"))?,
        };
        let utc = match self.utc_offset.trim() {
            "" => 0,
            t => parse_utc_offset(t)
                .ok_or_else(|| format!("UTC offset: cannot read {t:?} (e.g. +07:00)"))?,
        };
        let wf = self.weather_file.trim();
        Ok(SetSiteLocation {
            latitude: Some(deg(&self.latitude, "latitude")?),
            longitude: Some(deg(&self.longitude, "longitude")?),
            elevation: Some(elevation),
            utc_offset_minutes: Some(utc),
            place: Some(self.place.trim().to_string()),
            weather_file: (!wf.is_empty()).then(|| wf.to_string()),
            clear: false,
        })
    }
}

fn spec_label(spec: QuantitySpec) -> &'static str {
    use QuantitySpec as Q;
    match spec {
        Q::Length => "Length",
        Q::Area => "Area",
        Q::Volume => "Volume",
        Q::Angle => "Angle",
        Q::Mass => "Mass",
        Q::Density => "Density",
        Q::ThermalConductivity => "Thermal conductivity",
        Q::ThermalResistance => "Thermal resistance",
        Q::UValue => "U-value",
        Q::HeatCapacity => "Specific heat",
        Q::Temperature => "Temperature",
        Q::CarbonMass => "Carbon (GWP)",
        other => other.id(),
    }
}

fn unit_label(symbol: &str) -> &str {
    if symbol.is_empty() {
        "(number)"
    } else {
        symbol
    }
}

/// Degrees with up to six decimals and no trailing zeros (`16.05`).
fn trim_deg(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

/// `"+07:00"` for 420 minutes, `"-03:30"` for −210.
pub fn format_utc_offset(minutes: i32) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let m = minutes.unsigned_abs();
    format!("{sign}{:02}:{:02}", m / 60, m % 60)
}

/// Read `"+7"`, `"7"`, `"+07:00"`, `"-3:30"`, `"5.5"` or `"UTC+7"` as
/// minutes. `None` if unreadable or outside −12:00..+14:00.
pub fn parse_utc_offset(text: &str) -> Option<i32> {
    let t = text.trim();
    let t = t
        .strip_prefix("UTC")
        .or_else(|| t.strip_prefix("GMT"))
        .unwrap_or(t)
        .trim();
    let (neg, body) = match t.strip_prefix('-').or_else(|| t.strip_prefix('−')) {
        Some(b) => (true, b),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let minutes = match body.split_once(':') {
        Some((h, m)) => {
            let h: u32 = h.trim().parse().ok()?;
            let m: u32 = m.trim().parse().ok()?;
            if m >= 60 {
                return None;
            }
            (h * 60 + m) as i32
        }
        None => {
            let h: f64 = body.trim().parse().ok()?;
            if !h.is_finite() || h < 0.0 {
                return None;
            }
            (h * 60.0).round() as i32
        }
    };
    let minutes = if neg { -minutes } else { minutes };
    (-12 * 60..=14 * 60).contains(&minutes).then_some(minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_offsets_round_trip() {
        assert_eq!(parse_utc_offset("+07:00"), Some(420));
        assert_eq!(parse_utc_offset("7"), Some(420));
        assert_eq!(parse_utc_offset("UTC+7"), Some(420));
        assert_eq!(parse_utc_offset("-3:30"), Some(-210));
        assert_eq!(parse_utc_offset("5.5"), Some(330));
        assert_eq!(parse_utc_offset("+15"), None);
        assert_eq!(parse_utc_offset("7:75"), None);
        assert_eq!(parse_utc_offset("abc"), None);
        assert_eq!(format_utc_offset(420), "+07:00");
        assert_eq!(format_utc_offset(-210), "-03:30");
        assert_eq!(format_utc_offset(0), "+00:00");
        assert_eq!(trim_deg(16.05), "16.05");
        assert_eq!(trim_deg(-0.0000001), "0");
    }

    #[test]
    fn location_text_becomes_one_command() {
        let mut d = ProjectSettingsDraft {
            latitude: "16.05".into(),
            longitude: "108.2°".into(),
            elevation: "5m".into(),
            utc_offset: "+07:00".into(),
            place: " Đà Nẵng ".into(),
            ..Default::default()
        };
        let s = ProjectSettings::default();
        let c = d.location_command(&s);
        let c = c.as_ref().ok();
        assert!(c.is_some_and(
            |c| (c.latitude.map(|a| a.deg()).unwrap_or(0.0) - 16.05).abs() < 1e-9
                && c.elevation.is_some_and(|l| (l.mm() - 5000.0).abs() < 1e-9)
                && c.utc_offset_minutes == Some(420)
                && c.place.as_deref() == Some("Đà Nẵng")
                && c.weather_file.is_none()
        ));
        d.latitude = "north".into();
        assert!(d.location_command(&s).is_err());
    }
}
