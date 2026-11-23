//! `set_parameter` values at the edge (ADR 0004 §2): agents may give
//! quantities with units, which are converted here to storage units
//! (`ParamValue::Quantity`, SI; mm for length, rad for angle).

use ferro_core::model::ParameterDataType;
use ferro_core::{Document, QuantitySpec};
use serde_json::{json, Value};

const TEXT_HINT: &str = "for text use {\"type\": \"text\", \"value\": \"...\"}";

/// Convert an agent `set_parameter` value to the kernel's `ParamValue` JSON.
///
/// Accepted forms (the key's definition, if any, gives the expected type):
/// - `{"type": ..., "value": ...}`: the kernel form, passed through
///   unchanged (quantities in storage units).
/// - `"0.8 W/mK"`, `"1200 kg/m3"`, `"35 MPa"`, `"20 °C"`, `"3m"`: a quantity
///   with a unit. With a quantity definition, a bare number is in the spec's
///   default display unit. With a text definition, the string is the text.
/// - `{"spec": "density", "value": 1200, "unit": "kg/m3"}`: `unit` optional
///   (absent = storage unit); `value` may also be a string with a unit.
/// - a number: `number` (or storage units under a quantity definition,
///   `integer` under an integer definition); `true` / `false`: `bool`;
///   an array of strings: `tags`.
///
/// Fails with a message an agent can act on (unknown unit, missing unit,
/// unknown spec, wrong shape).
pub fn param_value(doc: &Document, key: &str, value: &Value) -> Result<Value, String> {
    let expected = doc.parameter_definition(key).map(|d| d.data_type);
    let expected_spec = match expected {
        Some(ParameterDataType::Quantity(s)) => Some(s),
        _ => None,
    };
    let quantity = |s: QuantitySpec, v: f64| json!({"type": "quantity", "value": {"spec": s.id(), "value": v}});
    match value {
        Value::Object(o) if o.contains_key("type") => Ok(value.clone()),
        Value::Object(o) if o.contains_key("spec") => {
            let id = o.get("spec").and_then(Value::as_str).unwrap_or_default();
            let spec = QuantitySpec::from_id(id).ok_or_else(|| {
                let ids: Vec<&str> = QuantitySpec::ALL.iter().map(|s| s.id()).collect();
                format!("unknown spec {id:?}; use one of: {}", ids.join(", "))
            })?;
            let unit = match o.get("unit") {
                None | Some(Value::Null) => "",
                Some(Value::String(u)) => u.as_str(),
                Some(_) => return Err("unit must be a string such as \"W/mK\"".into()),
            };
            let stored = match o.get("value") {
                Some(Value::Number(n)) => {
                    let n = n.as_f64().ok_or("value must be a number")?;
                    spec.to_storage(n, unit).map_err(|e| e.to_string())?
                }
                Some(Value::String(s)) if unit.is_empty() => {
                    QuantitySpec::parse(s, Some(spec))
                        .map_err(|e| e.to_string())?
                        .1
                }
                Some(Value::String(_)) => {
                    return Err("give value as a number when unit is set".into())
                }
                _ => return Err("value must be a number or a string with a unit".into()),
            };
            Ok(quantity(spec, stored))
        }
        Value::Object(_) => Err(
            "value must be {\"type\": ..., \"value\": ...}, {\"spec\": ..., \"value\": ..., \"unit\": ...} or a string such as \"0.8 W/mK\"".into(),
        ),
        Value::String(s) => match expected {
            Some(ParameterDataType::Text) => Ok(json!({"type": "text", "value": s})),
            Some(ParameterDataType::Quantity(_)) | None => {
                let (spec, v) = QuantitySpec::parse(s, expected_spec).map_err(|e| {
                    if expected.is_none() {
                        format!("{e}; {TEXT_HINT}")
                    } else {
                        e.to_string()
                    }
                })?;
                Ok(quantity(spec, v))
            }
            Some(t) => Err(format!(
                "{key} holds {}; pass {{\"type\": ..., \"value\": ...}}",
                t.describe()
            )),
        },
        Value::Number(n) => match expected {
            Some(ParameterDataType::Integer) => n
                .as_i64()
                .map(|i| json!({"type": "integer", "value": i}))
                .ok_or_else(|| format!("{key} holds integers")),
            Some(ParameterDataType::Quantity(s)) => {
                let v = n.as_f64().ok_or("value must be a number")?;
                Ok(quantity(s, v))
            }
            _ => Ok(json!({"type": "number", "value": n})),
        },
        Value::Bool(b) => Ok(json!({"type": "bool", "value": b})),
        Value::Array(a) if a.iter().all(Value::is_string) => Ok(json!({"type": "tags", "value": a})),
        _ => Err("unsupported parameter value".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_core::math::tol;
    use ferro_core::model::keys;
    use ferro_core::ParamValue;

    fn q(doc: &Document, key: &str, v: Value) -> (QuantitySpec, f64) {
        let j = param_value(doc, key, &v).unwrap();
        match serde_json::from_value::<ParamValue>(j).unwrap() {
            ParamValue::Quantity { spec, value } => (spec, value),
            other => panic!("not a quantity: {other:?}"),
        }
    }

    #[test]
    fn text_quantities_convert_to_storage_units() {
        let d = Document::new("T");
        let (s, v) = q(&d, "acme.a.k", json!("0.8 W/mK"));
        assert_eq!(s, QuantitySpec::ThermalConductivity);
        assert!(tol::eq_quantity(v, 0.8));
        let (s, v) = q(&d, "acme.a.rho", json!("1200 kg/m3"));
        assert_eq!(s, QuantitySpec::Density);
        assert!(tol::eq_quantity(v, 1200.0));
        let (s, v) = q(&d, "acme.a.fck", json!("35 MPa"));
        assert_eq!(s, QuantitySpec::Stress);
        assert!(tol::eq_quantity(v, 35e6));
        let (s, v) = q(&d, "acme.a.len", json!("3m"));
        assert_eq!(s, QuantitySpec::Length);
        assert!(tol::eq_len(v, 3000.0));
    }

    #[test]
    fn json_quantities_with_units_convert() {
        let d = Document::new("T");
        let (s, v) = q(
            &d,
            "acme.a.x",
            json!({"spec": "stress", "value": 35, "unit": "MPa"}),
        );
        assert_eq!(s, QuantitySpec::Stress);
        assert!(tol::eq_quantity(v, 35e6));
        // No unit = storage unit.
        let (_, v) = q(
            &d,
            "acme.a.x",
            json!({"spec": "temperature", "value": 293.15}),
        );
        assert!(tol::eq_quantity(v, 293.15));
        let (_, v) = q(
            &d,
            "acme.a.x",
            json!({"spec": "temperature", "value": "20 °C"}),
        );
        assert!(tol::eq_quantity(v, 293.15));
    }

    #[test]
    fn bad_values_explain_themselves() {
        let d = Document::new("T");
        let e = |v: Value| param_value(&d, "acme.a.x", &v).unwrap_err();
        assert!(e(json!("hello")).contains("text"));
        assert!(e(json!("0.8")).contains("unit"));
        assert!(e(json!({"spec": "furlong", "value": 1})).contains("thermal_conductivity"));
        assert!(e(json!({"spec": "density", "value": 1, "unit": "MPa"})).contains("kg/m³"));
        assert!(e(json!({"spec": "density", "value": "1", "unit": "kg/m3"})).contains("number"));
        assert!(e(json!({"value": 1})).contains("type"));
        assert!(e(Value::Null).contains("unsupported"));
    }

    #[test]
    fn definitions_guide_plain_values() {
        let d = Document::new("T");
        // Comments are text: a string stays text even if it looks like "3m".
        assert_eq!(
            param_value(&d, keys::COMMENTS, &json!("3m")).unwrap(),
            json!({"type": "text", "value": "3m"})
        );
        assert_eq!(
            param_value(&d, keys::LOD, &json!(300)).unwrap(),
            json!({"type": "integer", "value": 300})
        );
        assert!(param_value(&d, keys::LOD, &json!(2.5)).is_err());
        assert!(param_value(&d, keys::LOD, &json!("300")).is_err());
        // Kernel form passes through.
        let k = json!({"type": "length", "value": 200.0});
        assert_eq!(param_value(&d, "acme.a.x", &k).unwrap(), k);
        assert_eq!(
            param_value(&d, "acme.a.x", &json!(["a", "b"])).unwrap(),
            json!({"type": "tags", "value": ["a", "b"]})
        );
    }
}
