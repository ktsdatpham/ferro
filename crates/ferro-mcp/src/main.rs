//! ferro-mcp: lets any MCP client (Claude Desktop, Claude Code, Cursor, ...)
//! model in Ferro. Transport: newline-delimited JSON-RPC 2.0 over stdio.
//!
//!   cargo run -p ferro-mcp
//!
//! Every kernel command is a tool (D-17). Logs go to stderr.

mod tools;

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use tools::Server;

const PROTOCOL_VERSION: &str = "2025-06-18";

fn handle(server: &mut Server, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned()?; // notifications get no reply
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    let result = match method {
        "initialize" => {
            if let Some(name) = params["clientInfo"]["name"].as_str() {
                server.agent = ferro_core::ops::Actor::agent(
                    name,
                    params["clientInfo"]["version"]
                        .as_str()
                        .unwrap_or("unknown"),
                );
            }
            Ok(json!({
                "protocolVersion": params.get("protocolVersion").cloned().unwrap_or_else(|| json!(PROTOCOL_VERSION)),
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "ferro", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Ferro is an open-source, AI-native 3D design tool for architects. \
                    Units: millimetres unless a unit is given (\"3m\"); angles in degrees; Z is up. \
                    Typical flow: create_level -> create_box / create_rectangle + push_pull_face -> \
                    set_category -> list_elements / get_element. Every change is one undo step and \
                    is recorded with your client name. Save with save_document (.fro)."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::list() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let (body, is_error) = match server.call(name, &args) {
                Ok(v) => (v, false),
                Err(v) => (v, true),
            };
            Ok(
                json!({ "content": [{ "type": "text", "text": body.to_string() }], "isError": is_error }),
            )
        }
        _ => Err(json!({ "code": -32601, "message": format!("method not found: {method}") })),
    };

    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

fn main() {
    let mut server = Server::new();
    let stdin = io::stdin();
    let mut out = io::stdout().lock();
    eprintln!("ferro-mcp {} ready on stdio", env!("CARGO_PKG_VERSION"));
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => handle(&mut server, &req),
            Err(e) => Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("parse error: {e}")}}),
            ),
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

    fn call(s: &mut Server, name: &str, args: Value) -> Value {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}});
        let r = handle(s, &req).unwrap()["result"].clone();
        let text = r["content"][0]["text"].as_str().unwrap().to_string();
        json!({"is_error": r["isError"], "body": serde_json::from_str::<Value>(&text).unwrap()})
    }

    #[test]
    fn every_command_has_a_tool_with_the_same_name() {
        let names: Vec<String> = tools::list()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        for c in ferro_core::AnyCommand::NAMES {
            assert!(names.iter().any(|n| n == c), "missing tool for {c}");
        }
    }

    #[test]
    fn agent_models_a_house_with_units_and_undo() {
        let mut s = Server::new();
        let r = call(
            &mut s,
            "create_level",
            json!({"name":"L2","elevation":"3.3m"}),
        );
        assert_eq!(r["is_error"], false, "{r}");
        let r = call(
            &mut s,
            "create_box",
            json!({"origin":[0,0,0],"width":"4m","depth":"16m","height":"3300mm","level":1,"name":"Tầng 2"}),
        );
        assert_eq!(r["is_error"], false, "{r}");
        let id = r["body"]["result"]["element"].as_u64().unwrap();
        let d = call(&mut s, "get_element", json!({"element": id}));
        let faces = d["body"]["faces"].as_array().unwrap();
        assert_eq!(faces.len(), 6);
        let top = faces.iter().find(|f| f["normal"][2] == 1.0).unwrap()["face"]
            .as_u64()
            .unwrap();
        let r = call(
            &mut s,
            "push_pull_face",
            json!({"element": id, "face": top, "distance": "50cm"}),
        );
        assert_eq!(r["body"]["result"]["mode"], "stretch", "{r}");
        let l = call(&mut s, "list_elements", json!({"category":"forms"}));
        assert_eq!(l["body"]["elements"][0]["volume_m3"], 4.0 * 16.0 * 3.8);
        call(&mut s, "undo", json!({}));
        let l = call(&mut s, "list_elements", json!({"category":"forms"}));
        assert_eq!(l["body"]["elements"][0]["volume_m3"], 4.0 * 16.0 * 3.3);
        let r = call(
            &mut s,
            "rotate_elements",
            json!({"elements":[id],"center":[0,0,0],"angle":90}),
        );
        assert_eq!(r["is_error"], false);
        let j = call(&mut s, "get_journal", json!({}));
        assert_eq!(j["body"][0]["actor"]["kind"], "agent");
    }

    #[test]
    fn failures_are_reported_and_change_nothing() {
        let mut s = Server::new();
        let r = call(
            &mut s,
            "create_box",
            json!({"origin":[0,0,0],"width":"abc","depth":1,"height":1}),
        );
        assert_eq!(r["is_error"], true);
        let r = call(
            &mut s,
            "push_pull_face",
            json!({"element": 99, "face": 1, "distance": 100}),
        );
        assert_eq!(r["body"]["failures"][0]["code"], "ferro.model.not_found");
        assert_eq!(s.session.doc().len(), 0);
    }

    #[test]
    fn save_and_open_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ferro-mcp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.fro");
        let mut s = Server::new();
        call(
            &mut s,
            "create_box",
            json!({"origin":[0,0,0],"width":1000,"depth":1000,"height":1000}),
        );
        let r = call(&mut s, "save_document", json!({"path": path}));
        assert_eq!(r["is_error"], false, "{r}");
        let mut s2 = Server::new();
        let r = call(&mut s2, "open_document", json!({"path": path}));
        assert_eq!(r["body"]["elements"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn notifications_get_no_reply() {
        let mut s = Server::new();
        assert!(handle(
            &mut s,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none());
    }
}
