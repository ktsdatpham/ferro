//! Scripts: a JSON array of tool calls run in order in a headless session.
//!
//! ```json
//! [
//!   {"tool": "create_level", "args": {"name": "L1", "elevation": 0}, "id": "l1"},
//!   {"tool": "create_walls", "args": {"points": [[0,0],["8m",0],["8m","6m"],[0,"6m"]],
//!                                     "closed": true, "level": {"$ref": "l1.result.element"}}}
//! ]
//! ```
//!
//! Tool names and arguments are exactly those of MCP (`ferro-cli tools`).
//! An argument `{"$ref": "<step>.<path>"}` is replaced by a value from the
//! result of an earlier successful step: `<step>` is its `id` or its 0-based
//! index, `<path>` walks object keys and array indices (`result.elements.0`).

use crate::{codes, CliError, Io};
use ferro_core::ops::Actor;
use ferro_mcp::tools::Server;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

/// One tool call of a script.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Optional name other steps use in `{"$ref": "<id>.…"}`.
    pub id: Option<String>,
    /// Tool name (same as MCP).
    pub tool: String,
    /// Tool arguments (same as MCP; units as strings allowed).
    pub args: Value,
}

/// How `run` treats failures.
#[derive(Debug, Default, Clone, Copy)]
pub struct RunOpts {
    /// Continue after a failed call (and still write the outputs).
    pub keep_going: bool,
    /// Run the whole script as one transaction group: all or nothing.
    pub atomic: bool,
}

/// The actor recorded in the journal for CLI calls.
pub fn actor() -> Actor {
    Actor::agent("ferro-cli", env!("CARGO_PKG_VERSION"))
}

/// Read a script from a file, or from stdin when `path` is `-`.
/// Fails with `ferro.io.file` when it cannot be read.
pub fn read_source(path: &str) -> Result<String, CliError> {
    let mut text = String::new();
    let r = if path == "-" {
        std::io::stdin().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(path).map(|t| text = t)
    };
    r.map_err(|e| CliError::new("ferro.io.file", format!("cannot read {path}: {e}")))?;
    Ok(text)
}

/// Parse tool arguments given on the command line: a JSON object.
/// Fails with `ferro.cli.script`.
pub fn parse_args(text: &str) -> Result<Value, CliError> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| CliError::new(codes::SCRIPT, format!("arguments are not JSON: {e}")))?;
    if !v.is_object() {
        return Err(CliError::new(
            codes::SCRIPT,
            "arguments must be a JSON object",
        ));
    }
    Ok(v)
}

/// Parse a script (JSON array of `{"tool", "args"?, "id"?}`). Unknown keys,
/// duplicate or numeric ids and wrong types fail with `ferro.cli.script`.
pub fn parse(text: &str) -> Result<Vec<Step>, CliError> {
    let bad = |m: String| CliError::new(codes::SCRIPT, m);
    let v: Value =
        serde_json::from_str(text).map_err(|e| bad(format!("script is not JSON: {e}")))?;
    let Value::Array(items) = v else {
        return Err(bad(
            "a script is a JSON array of {\"tool\", \"args\"} objects".into(),
        ));
    };
    let mut steps = Vec::with_capacity(items.len());
    let mut seen = BTreeMap::new();
    for (i, item) in items.into_iter().enumerate() {
        let Value::Object(o) = item else {
            return Err(bad(format!("step {i}: expected an object")));
        };
        if let Some(k) = o
            .keys()
            .find(|k| !["tool", "args", "id"].contains(&k.as_str()))
        {
            return Err(bad(format!(
                "step {i}: unknown key {k:?} (use tool, args, id)"
            )));
        }
        let tool = match o.get("tool") {
            Some(Value::String(t)) if !t.is_empty() => t.clone(),
            _ => return Err(bad(format!("step {i}: \"tool\" must be a tool name"))),
        };
        let args = match o.get("args") {
            None | Some(Value::Null) => json!({}),
            Some(a @ Value::Object(_)) => a.clone(),
            Some(_) => return Err(bad(format!("step {i}: \"args\" must be an object"))),
        };
        let id = match o.get("id") {
            None | Some(Value::Null) => None,
            Some(Value::String(s))
                if !s.is_empty() && s.parse::<usize>().is_err() && !s.contains('.') =>
            {
                if seen.insert(s.clone(), i).is_some() {
                    return Err(bad(format!("step {i}: id {s:?} is used twice")));
                }
                Some(s.clone())
            }
            Some(_) => {
                return Err(bad(format!(
                    "step {i}: \"id\" must be a non-numeric name without dots"
                )))
            }
        };
        steps.push(Step { id, tool, args });
    }
    Ok(steps)
}

/// Runs steps against a server and remembers results for `$ref`.
#[derive(Debug, Default)]
pub struct Runner {
    results: Vec<Option<Value>>,
    ids: BTreeMap<String, usize>,
}

impl Runner {
    /// Run the next step (steps must be given in order). Ok = tool result,
    /// Err = tool error (`{"error", "failures"?}`); a bad `$ref` is an error
    /// with code `ferro.cli.bad_ref` and calls nothing.
    pub fn step(&mut self, server: &mut Server, step: &Step) -> Result<Value, Value> {
        let index = self.results.len();
        if let Some(id) = &step.id {
            self.ids.insert(id.clone(), index);
        }
        let r = match self.resolve(&step.args) {
            Ok(args) => server.call(&step.tool, &args),
            Err(m) => Err(json!({"error": m, "failures": [failure(codes::BAD_REF, &m)]})),
        };
        self.results.push(r.as_ref().ok().cloned());
        r
    }

    fn resolve(&self, v: &Value) -> Result<Value, String> {
        match v {
            Value::Object(o) if o.len() == 1 && o.contains_key("$ref") => {
                let Some(path) = o.get("$ref").and_then(Value::as_str) else {
                    return Err("\"$ref\" must be a string".into());
                };
                self.lookup(path)
            }
            Value::Object(o) => {
                let mut m = Map::new();
                for (k, x) in o {
                    m.insert(k.clone(), self.resolve(x)?);
                }
                Ok(Value::Object(m))
            }
            Value::Array(a) => a
                .iter()
                .map(|x| self.resolve(x))
                .collect::<Result<_, _>>()
                .map(Value::Array),
            other => Ok(other.clone()),
        }
    }

    fn lookup(&self, path: &str) -> Result<Value, String> {
        let mut parts = path.split('.');
        let head = parts.next().unwrap_or_default();
        let index = match head.parse::<usize>() {
            Ok(i) => Some(i),
            Err(_) => self.ids.get(head).copied(),
        };
        // Results of earlier steps only (the current one is not pushed yet).
        let Some(i) = index.filter(|i| *i < self.results.len()) else {
            return Err(format!("$ref {path:?}: no earlier step {head:?}"));
        };
        let Some(mut v) = self.results.get(i).and_then(Option::as_ref) else {
            return Err(format!("$ref {path:?}: step {head:?} failed"));
        };
        for p in parts {
            let next = match v {
                Value::Array(a) => p.parse::<usize>().ok().and_then(|k| a.get(k)),
                Value::Object(o) => o.get(p),
                _ => None,
            };
            let Some(n) = next else {
                return Err(format!(
                    "$ref {path:?}: no {p:?} in the result of step {head:?}"
                ));
            };
            v = n;
        }
        Ok(v.clone())
    }
}

/// A failure entry in the kernel's JSON shape.
pub fn failure(code: &str, message: &str) -> Value {
    json!({"severity": "error", "code": code, "message": message, "elements": []})
}

/// One output line for a call: `{tool, ok, result}` or
/// `{tool, ok: false, error, failures}`. Errors without kernel failures get
/// one `ferro.cli.tool_error` failure, so every failure has a code.
pub fn line(step: Option<usize>, tool: &str, r: &Result<Value, Value>) -> Value {
    let mut v = match r {
        Ok(result) => json!({"tool": tool, "ok": true, "result": result}),
        Err(e) => {
            let message = e
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("tool failed")
                .to_string();
            let failures = match e.get("failures") {
                Some(Value::Array(f)) if !f.is_empty() => Value::Array(f.clone()),
                _ => json!([failure(codes::TOOL_ERROR, &message)]),
            };
            json!({"tool": tool, "ok": false, "error": message, "failures": failures})
        }
    };
    if let Some(i) = step {
        v["step"] = json!(i);
    }
    v
}

/// Open a model into a headless server. Fails with the file's `ferro.io.*`
/// code.
pub fn open(path: &Path) -> Result<Server, CliError> {
    let mut server = Server::headless(actor());
    open_into(&mut server, path)?;
    Ok(server)
}

fn open_into(server: &mut Server, path: &Path) -> Result<(), CliError> {
    match server.call("open_document", &json!({"path": path})) {
        Ok(_) => Ok(()),
        Err(e) => {
            let code = e["failures"][0]["code"].as_str().unwrap_or("ferro.io.file");
            let msg = e["error"].as_str().unwrap_or("cannot open");
            Err(CliError::new(code, format!("{}: {msg}", path.display())))
        }
    }
}

/// A headless server with `--in` opened (if given).
pub fn session(input: Option<&Path>) -> Result<Server, CliError> {
    let mut server = Server::headless(actor());
    if let Some(p) = input {
        open_into(&mut server, p)?;
    }
    Ok(server)
}

/// Write the outputs; prints one line per file. Returns the number that failed.
fn write_outputs(server: &mut Server, io: &Io) -> usize {
    let mut failed = 0;
    for (tool, args) in io.output_calls() {
        let r = server.call(tool, &args);
        failed += usize::from(r.is_err());
        crate::outln!("{}", line(None, tool, &r));
    }
    failed
}

/// `ferro-cli run`: prints one JSON line per call, then
/// `{"summary": {calls, failed, elements, rolled_back, ok}}`. Exit 1 when a
/// call failed, 2 when `--in` cannot be opened.
pub fn run(steps: &[Step], io: &Io, opts: &RunOpts) -> ExitCode {
    let mut server = match session(io.input.as_deref()) {
        Ok(s) => s,
        Err(e) => return crate::report(&e),
    };
    if opts.atomic {
        let ctx = server.ctx();
        server.session.begin_group("ferro-cli script", ctx);
    }
    let mut runner = Runner::default();
    let mut failed = 0;
    let mut calls = 0;
    for (i, step) in steps.iter().enumerate() {
        let r = runner.step(&mut server, step);
        calls += 1;
        crate::outln!("{}", line(Some(i), &step.tool, &r));
        if r.is_err() {
            failed += 1;
            if !opts.keep_going {
                break;
            }
        }
    }
    let mut rolled_back = false;
    if opts.atomic {
        if failed > 0 {
            rolled_back = server.session.cancel_group().is_some();
        } else {
            server.session.end_group();
        }
    }
    if failed == 0 || opts.keep_going {
        failed += write_outputs(&mut server, io);
    }
    crate::outln!(
        "{}",
        json!({"summary": {"calls": calls, "failed": failed, "elements": server.session.doc().len(), "rolled_back": rolled_back, "ok": failed == 0}})
    );
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// `ferro-cli call`: one call, one line, then the outputs (when it succeeded).
pub fn call_one(tool: &str, args: Value, io: &Io) -> ExitCode {
    let mut server = match session(io.input.as_deref()) {
        Ok(s) => s,
        Err(e) => return crate::report(&e),
    };
    let step = Step {
        id: None,
        tool: tool.into(),
        args,
    };
    let r = Runner::default().step(&mut server, &step);
    crate::outln!("{}", line(None, tool, &r));
    let mut failed = usize::from(r.is_err());
    if failed == 0 {
        failed += write_outputs(&mut server, io);
    }
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_steps_and_rejects_bad_shapes() {
        let s = parse(r#"[{"tool":"create_level","args":{"name":"L1","elevation":0},"id":"l1"},{"tool":"undo"}]"#)
            .unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].id.as_deref(), Some("l1"));
        assert_eq!(s[1].args, json!({}));
        for bad in [
            "",
            "{",
            "{}",
            "[1]",
            r#"[{"args":{}}]"#,
            r#"[{"tool":"x","arg":{}}]"#,
            r#"[{"tool":"x","args":[1]}]"#,
            r#"[{"tool":"x","id":"3"}]"#,
            r#"[{"tool":"x","id":"a"},{"tool":"y","id":"a"}]"#,
        ] {
            assert_eq!(parse(bad).unwrap_err().code, codes::SCRIPT, "{bad}");
        }
        assert!(parse("[]").unwrap().is_empty());
    }

    #[test]
    fn refs_resolve_by_id_and_index_and_fail_cleanly() {
        let mut server = Server::headless(actor());
        let mut r = Runner::default();
        let steps = parse(
            r#"[
            {"tool":"create_level","args":{"name":"L1","elevation":0},"id":"l1"},
            {"tool":"create_box","args":{"origin":[0,0,0],"width":1000,"depth":1000,"height":1000,"level":{"$ref":"l1.result.element"}}},
            {"tool":"get_element","args":{"element":{"$ref":"1.result.element"}}},
            {"tool":"get_element","args":{"element":{"$ref":"l1.result.nothing"}}},
            {"tool":"get_element","args":{"element":{"$ref":"9.result"}}},
            {"tool":"get_element","args":{"element":{"$ref":5}}}
        ]"#,
        )
        .unwrap();
        for s in &steps[..3] {
            assert!(r.step(&mut server, s).is_ok());
        }
        for s in &steps[3..] {
            let e = r.step(&mut server, s).unwrap_err();
            assert_eq!(e["failures"][0]["code"], codes::BAD_REF, "{e}");
        }
        // A step cannot refer to itself.
        let own = parse(r#"[{"tool":"get_element","args":{"element":{"$ref":"6.x"}}}]"#).unwrap();
        assert!(r.step(&mut server, &own[0]).is_err());
    }

    #[test]
    fn every_failed_line_has_a_code() {
        let l = line(
            Some(0),
            "create_box",
            &Err(json!({"error": "cannot read length"})),
        );
        assert_eq!(l["failures"][0]["code"], codes::TOOL_ERROR);
        assert_eq!(l["ok"], false);
        let l = line(None, "x", &Ok(json!({"a": 1})));
        assert!(l.get("step").is_none());
        assert_eq!(l["result"]["a"], 1);
    }
}
