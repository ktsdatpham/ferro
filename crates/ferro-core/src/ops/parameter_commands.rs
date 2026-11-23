//! Commands on parameter definitions (ADR 0004 §2.3). Each `NAME` is also
//! the MCP tool name.

use super::commands::{Command, CommandOutput};
use super::{CommandError, Transaction};
use crate::model::{
    builtin_definitions, keys, IfcPropertyMapping, ParameterBinding, ParameterDataType,
    ParameterDefinition, ParameterGroup, Uid,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Add or replace a project parameter definition (Revit: create a shared /
/// project parameter and bind it). Built-in `ferro.*` keys cannot be
/// defined. Without `uid`, an existing definition keeps its `Uid` and a new
/// one gets `ParameterDefinition::uid_for_key(key)`; pass the Revit shared
/// parameter GUID to match an existing Revit parameter.
///
/// Fails with `ferro.model.invalid_parameter_key`,
/// `ferro.model.builtin_parameter_definition`,
/// `ferro.model.invalid_parameter_definition` (empty IFC names),
/// `ferro.model.duplicate_uid` (another definition has the `Uid`) or
/// `ferro.model.parameter_type_mismatch` (elements hold values of another
/// type for this key; they are listed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetParameterDefinition {
    pub key: String,
    pub data_type: ParameterDataType,
    #[serde(default)]
    pub group: ParameterGroup,
    #[serde(default)]
    pub binding: ParameterBinding,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub uid: Option<Uid>,
    #[serde(default)]
    pub ifc: Option<IfcPropertyMapping>,
}

fn err(code: &str, msg: impl Into<String>) -> CommandError {
    CommandError::new(code, msg)
}

impl Command for SetParameterDefinition {
    const NAME: &'static str = "set_parameter_definition";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        if !keys::is_valid(&self.key) {
            return Err(err(
                "ferro.model.invalid_parameter_key",
                format!("{:?} is not a valid key (namespace.group.name)", self.key),
            ));
        }
        if ParameterDefinition::is_builtin_key(&self.key) {
            return Err(err(
                "ferro.model.builtin_parameter_definition",
                format!(
                    "{} is in the built-in ferro namespace; use your own namespace",
                    self.key
                ),
            ));
        }
        if let Some(m) = &self.ifc {
            if m.property_set.trim().is_empty() || m.property.trim().is_empty() {
                return Err(err(
                    "ferro.model.invalid_parameter_definition",
                    "IFC property set and property names must not be empty",
                ));
            }
        }
        let project = &tx.doc().project().parameter_definitions;
        let existing = project.iter().find(|d| d.key == self.key);
        let uid = self
            .uid
            .or(existing.map(|d| d.uid))
            .unwrap_or_else(|| ParameterDefinition::uid_for_key(&self.key));
        if let Some(other) = project
            .iter()
            .cloned()
            .chain(builtin_definitions())
            .find(|d| d.uid == uid && d.key != self.key)
        {
            return Err(err(
                "ferro.model.duplicate_uid",
                format!("{} already uses uid {uid}", other.key),
            ));
        }
        let misfits: Vec<_> = tx
            .doc()
            .elements()
            .filter(|e| {
                e.params
                    .get(&self.key)
                    .is_some_and(|v| !self.data_type.accepts(v))
            })
            .map(|e| e.id)
            .collect();
        if !misfits.is_empty() {
            let mut e = err(
                "ferro.model.parameter_type_mismatch",
                format!(
                    "{} element(s) hold {} values that are not {}",
                    misfits.len(),
                    self.key,
                    self.data_type.describe()
                ),
            );
            e.elements = misfits;
            return Err(e);
        }
        tx.put_parameter_definition(ParameterDefinition {
            uid,
            key: self.key.clone(),
            data_type: self.data_type,
            group: self.group,
            binding: self.binding,
            description: self.description.clone(),
            read_only: false,
            revit_builtin: None,
            ifc: self.ifc.clone(),
        });
        Ok(json!({ "key": self.key, "uid": uid.to_string() }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::math::QuantitySpec;
    use crate::model::{keys, Document, ElementId, ParamValue, ParameterDefinition};
    use serde_json::{json, Value};

    fn session() -> Session {
        Session::new(Document::new("T"), Box::new(SequentialIds::new(1)))
    }

    fn try_run(s: &mut Session, tool: &str, args: Value) -> Result<Outcome, Vec<Failure>> {
        s.execute(
            AnyCommand::from_tool(tool, &args).expect("args"),
            &ExecContext::user(),
        )
    }

    fn code(r: Result<Outcome, Vec<Failure>>) -> String {
        r.expect_err("should fail")[0].code.clone()
    }

    fn boxed(s: &mut Session) -> ElementId {
        let o = try_run(
            s,
            "create_box",
            json!({"origin": {"x": 0, "y": 0, "z": 0}, "width": 1000, "depth": 1000, "height": 1000}),
        )
        .unwrap();
        serde_json::from_value(o.output["element"].clone()).unwrap()
    }

    fn set(
        s: &mut Session,
        e: ElementId,
        key: &str,
        value: Value,
    ) -> Result<Outcome, Vec<Failure>> {
        try_run(
            s,
            "set_parameter",
            json!({"element": e, "key": key, "value": value}),
        )
    }

    #[test]
    fn set_parameter_stores_a_quantity_in_storage_units() {
        let mut s = session();
        let b = boxed(&mut s);
        set(
            &mut s,
            b,
            "acme.energy.conductivity",
            json!({"type": "quantity", "value": {"spec": "thermal_conductivity", "value": 0.8}}),
        )
        .unwrap();
        let v = s
            .doc()
            .get(b)
            .unwrap()
            .param("acme.energy.conductivity")
            .cloned();
        assert_eq!(
            v,
            Some(ParamValue::Quantity {
                spec: QuantitySpec::ThermalConductivity,
                value: 0.8
            })
        );
    }

    #[test]
    fn set_parameter_checks_builtin_definitions() {
        let mut s = session();
        let b = boxed(&mut s);
        // Comments are text.
        assert_eq!(
            code(set(
                &mut s,
                b,
                keys::COMMENTS,
                json!({"type": "integer", "value": 3})
            )),
            "ferro.model.parameter_type_mismatch"
        );
        assert!(set(
            &mut s,
            b,
            keys::COMMENTS,
            json!({"type": "text", "value": "ok"})
        )
        .is_ok());
        // Read-only is reported before the type.
        assert_eq!(
            code(set(
                &mut s,
                b,
                keys::FORM_AREA,
                json!({"type": "text", "value": "x"})
            )),
            "ferro.model.read_only_parameter"
        );
    }

    #[test]
    fn project_definition_types_values_and_keeps_its_uid() {
        let mut s = session();
        let b = boxed(&mut s);
        let o = try_run(
            &mut s,
            "set_parameter_definition",
            json!({"key": "acme.energy.u_value", "data_type": {"quantity": "u_value"}, "group": "analysis", "binding": "type",
                   "ifc": {"property_set": "Pset_WallCommon", "property": "ThermalTransmittance"}}),
        )
        .unwrap();
        let uid = ParameterDefinition::uid_for_key("acme.energy.u_value");
        assert_eq!(o.output["uid"], json!(uid.to_string()));
        let d = s.doc().parameter_definition("acme.energy.u_value").unwrap();
        assert_eq!(d.uid, uid);
        // A density is not a U-value.
        assert_eq!(
            code(set(
                &mut s,
                b,
                "acme.energy.u_value",
                json!({"type": "quantity", "value": {"spec": "density", "value": 1.0}})
            )),
            "ferro.model.parameter_type_mismatch"
        );
        set(
            &mut s,
            b,
            "acme.energy.u_value",
            json!({"type": "quantity", "value": {"spec": "u_value", "value": 0.25}}),
        )
        .unwrap();
        // Re-defining keeps the uid; changing the type away from the stored
        // values is refused and lists the element.
        try_run(
            &mut s,
            "set_parameter_definition",
            json!({"key": "acme.energy.u_value", "data_type": {"quantity": "u_value"}, "description": "U"}),
        )
        .unwrap();
        assert_eq!(
            s.doc()
                .parameter_definition("acme.energy.u_value")
                .unwrap()
                .uid,
            uid
        );
        let f = try_run(
            &mut s,
            "set_parameter_definition",
            json!({"key": "acme.energy.u_value", "data_type": "text"}),
        )
        .unwrap_err();
        assert_eq!(f[0].code, "ferro.model.parameter_type_mismatch");
        assert_eq!(f[0].elements, vec![b]);
        // Undo removes the definition again.
        s.undo();
        s.undo();
        s.undo();
        assert!(s
            .doc()
            .parameter_definition("acme.energy.u_value")
            .is_none());
    }

    #[test]
    fn set_parameter_definition_rejects_bad_input() {
        let mut s = session();
        let def = |k: &str| json!({"key": k, "data_type": "text"});
        assert_eq!(
            code(try_run(&mut s, "set_parameter_definition", def("Bad"))),
            "ferro.model.invalid_parameter_key"
        );
        assert_eq!(
            code(try_run(
                &mut s,
                "set_parameter_definition",
                def(keys::COMMENTS)
            )),
            "ferro.model.builtin_parameter_definition"
        );
        assert_eq!(
            code(try_run(
                &mut s,
                "set_parameter_definition",
                json!({"key": "acme.a.b", "data_type": "text", "ifc": {"property_set": " ", "property": "X"}})
            )),
            "ferro.model.invalid_parameter_definition"
        );
        try_run(&mut s, "set_parameter_definition", def("acme.a.b")).unwrap();
        let taken = ParameterDefinition::uid_for_key("acme.a.b");
        assert_eq!(
            code(try_run(
                &mut s,
                "set_parameter_definition",
                json!({"key": "acme.a.c", "data_type": "text", "uid": taken.to_string()})
            )),
            "ferro.model.duplicate_uid"
        );
        let builtin = ParameterDefinition::uid_for_key(keys::COMMENTS);
        assert_eq!(
            code(try_run(
                &mut s,
                "set_parameter_definition",
                json!({"key": "acme.a.c", "data_type": "text", "uid": builtin.to_string()})
            )),
            "ferro.model.duplicate_uid"
        );
    }

    #[test]
    fn non_finite_values_are_rejected() {
        let mut s = session();
        let b = boxed(&mut s);
        let r = s.execute(
            AnyCommand::from(super::super::commands::SetParameter {
                element: b,
                key: "acme.a.b".into(),
                value: ParamValue::Quantity {
                    spec: QuantitySpec::Density,
                    value: f64::NAN,
                },
            }),
            &ExecContext::user(),
        );
        assert_eq!(code(r), "ferro.model.invalid_parameter_value");
    }

    #[test]
    fn definitions_persist_in_project_info_and_old_projects_load() {
        let mut s = session();
        try_run(
            &mut s,
            "set_parameter_definition",
            json!({"key": "acme.b.x", "data_type": {"quantity": "density"}}),
        )
        .unwrap();
        try_run(
            &mut s,
            "set_parameter_definition",
            json!({"key": "acme.a.x", "data_type": "bool"}),
        )
        .unwrap();
        let p = s.doc().project().clone();
        // Sorted by key, deterministic.
        let keys: Vec<_> = p
            .parameter_definitions
            .iter()
            .map(|d| d.key.as_str())
            .collect();
        assert_eq!(keys, ["acme.a.x", "acme.b.x"]);
        let text = serde_json::to_string(&p).unwrap();
        let back: crate::model::ProjectInfo = serde_json::from_str(&text).unwrap();
        assert_eq!(back, p);
        // An M1–M4 project has no definitions field.
        let old: crate::model::ProjectInfo = serde_json::from_value(json!({
            "name": "Old",
            "origins": serde_json::to_value(crate::model::ProjectOrigins::default()).unwrap()
        }))
        .unwrap();
        assert!(old.parameter_definitions.is_empty());
        assert!(!serde_json::to_string(&old)
            .unwrap()
            .contains("parameter_definitions"));
    }
}
