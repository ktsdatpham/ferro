//! phac-mcp: lets any MCP client (Claude Desktop, Claude Code, Cursor, ...)
//! model in Phac. Transport: newline-delimited JSON-RPC 2.0 over stdio.
//!
//!   cargo run -p phac-mcp
//!
//! Logs go to stderr; stdout is reserved for protocol messages.

mod tools;

use phac_core::Model;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const PROTOCOL_VERSION: &str = "2025-06-18";

fn handle(model: &mut Model, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    // Notifications (no id) never get a response.
    let id = id?;

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").cloned()
                .unwrap_or_else(|| json!(PROTOCOL_VERSION)),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "phac", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Phac is an open-source, SketchUp-style 3D design tool. \
                Units are millimetres unless a unit is given. Typical flow: \
                create_rectangle -> push_pull -> set_tags (category, level) -> list_entities."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::list() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let (text, is_error) = match tools::call(model, name, &args) {
                Ok(t) => (t, false),
                Err(e) => (e, true),
            };
            Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
        }
        _ => Err(json!({ "code": -32601, "message": format!("method not found: {method}") })),
    };

    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

fn main() {
    let mut model = Model::new();
    let stdin = io::stdin();
    let mut out = io::stdout().lock();
    eprintln!("phac-mcp {} ready on stdio", env!("CARGO_PKG_VERSION"));

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => handle(&mut model, &req),
            Err(e) => Some(json!({
                "jsonrpc": "2.0", "id": null,
                "error": { "code": -32700, "message": format!("parse error: {e}") }
            })),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(model: &mut Model, name: &str, args: Value) -> Value {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}});
        handle(model, &req).unwrap()["result"].clone()
    }

    #[test]
    fn agent_can_model_a_room() {
        let mut m = Model::new();
        let r = call(
            &mut m,
            "create_rectangle",
            json!({"width":"4m","depth":3000,"name":"Bedroom"}),
        );
        assert_eq!(r["isError"], false);
        let id: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        let id = id["id"].as_u64().unwrap();

        assert_eq!(
            call(&mut m, "push_pull", json!({"id":id,"distance":"2.8m"}))["isError"],
            false
        );
        assert_eq!(
            call(
                &mut m,
                "set_tags",
                json!({"id":id,"tags":{"category":"room"}})
            )["isError"],
            false
        );

        let list = call(&mut m, "list_entities", json!({}));
        let text = list["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"solid\"") && text.contains("Bedroom"));
    }

    #[test]
    fn notifications_get_no_reply() {
        let mut m = Model::new();
        assert!(handle(
            &mut m,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none());
    }
}
