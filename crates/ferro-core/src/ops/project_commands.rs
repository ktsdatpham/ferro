//! Project settings commands (M5-24): display units, site location,
//! currency. Each `NAME` is also the MCP tool name.

use super::commands::{Command, CommandOutput};
use super::{CommandError, Transaction};
use crate::math::{Angle, Length, QuantitySpec};
use crate::model::project_settings::{valid_currency, ProjectUnits, SiteLocation, UnitChoice};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Choose how a quantity is shown and typed, e.g. lengths in `m` with 2
/// decimals. `unit: null` returns the quantity to its default unit.
/// Fails with `ferro.model.invalid_unit`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetProjectUnits {
    pub spec: QuantitySpec,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub decimals: Option<u8>,
}

impl Command for SetProjectUnits {
    const NAME: &'static str = "set_project_units";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let choice = match &self.unit {
            Some(u) => {
                let c = UnitChoice {
                    unit: u.trim().to_string(),
                    decimals: self.decimals,
                };
                ProjectUnits::validate_choice(self.spec, &c)
                    .map_err(|m| CommandError::new("ferro.model.invalid_unit", m))?;
                // Store the unit's own symbol so files read the same everywhere.
                let symbol = self
                    .spec
                    .unit(&c.unit)
                    .map(|d| d.symbol.to_string())
                    .unwrap_or(c.unit);
                Some(UnitChoice {
                    unit: symbol,
                    decimals: c.decimals,
                })
            }
            None => None,
        };
        let spec = self.spec;
        tx.project_settings(|s| match choice {
            Some(c) => {
                s.units.units.insert(spec, c);
            }
            None => {
                s.units.units.remove(&spec);
            }
        });
        Ok(json!({ "spec": spec }))
    }
}

/// Set (or with `clear: true` remove) the project's place on Earth. Degrees
/// and metres at this edge; stored in radians and mm. Fails with
/// `ferro.model.invalid_location`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetSiteLocation {
    #[serde(default)]
    pub latitude: Option<Angle>,
    #[serde(default)]
    pub longitude: Option<Angle>,
    #[serde(default)]
    pub elevation: Option<Length>,
    #[serde(default)]
    pub utc_offset_minutes: Option<i32>,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub weather_file: Option<String>,
    #[serde(default)]
    pub clear: bool,
}

impl Command for SetSiteLocation {
    const NAME: &'static str = "set_site_location";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        if self.clear {
            tx.project_settings(|s| s.location = None);
            return Ok(json!({ "location": null }));
        }
        let old = tx.doc().project().settings.location.clone();
        let (Some(latitude), Some(longitude)) = (
            self.latitude.or(old.as_ref().map(|l| l.latitude)),
            self.longitude.or(old.as_ref().map(|l| l.longitude)),
        ) else {
            return Err(CommandError::new(
                "ferro.model.invalid_location",
                "give latitude and longitude",
            ));
        };
        let old = old.as_ref();
        let loc = SiteLocation {
            place: self
                .place
                .clone()
                .or_else(|| old.map(|l| l.place.clone()))
                .unwrap_or_default(),
            latitude,
            longitude,
            elevation: self
                .elevation
                .or(old.map(|l| l.elevation))
                .unwrap_or(Length::ZERO),
            utc_offset_minutes: self
                .utc_offset_minutes
                .or(old.map(|l| l.utc_offset_minutes))
                .unwrap_or(0),
            weather_file: self
                .weather_file
                .clone()
                .or_else(|| old.and_then(|l| l.weather_file.clone())),
        };
        loc.validate()
            .map_err(|m| CommandError::new("ferro.model.invalid_location", m))?;
        let out = json!({ "location": loc });
        tx.project_settings(|s| s.location = Some(loc));
        Ok(out)
    }
}

/// Set the currency used for costs (ISO 4217, e.g. `VND`); `null` clears it.
/// Fails with `ferro.model.invalid_currency`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetProjectCurrency {
    #[serde(default)]
    pub currency: Option<String>,
}

impl Command for SetProjectCurrency {
    const NAME: &'static str = "set_project_currency";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let c = self
            .currency
            .as_ref()
            .map(|c| c.trim().to_ascii_uppercase());
        if let Some(code) = &c {
            if !valid_currency(code) {
                return Err(CommandError::new(
                    "ferro.model.invalid_currency",
                    format!("`{code}` is not a three-letter ISO 4217 code"),
                ));
            }
        }
        let out = json!({ "currency": c });
        tx.project_settings(|s| s.currency = c);
        Ok(out)
    }
}
