//! ferro-mcp: lets any MCP client (Claude Desktop, Claude Code, Cursor, ...)
//! model in Ferro. Transport: newline-delimited JSON-RPC 2.0 over stdio.
//!
//!   cargo run -p ferro-mcp
//!   cargo run -p ferro-mcp -- --tools-json   # print the tool list and exit
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
                    Typical flow: create_level -> create_walls (closed, clockwise) -> create_opening \
                    -> create_floor; or massing with create_box + push_pull_face. Inspect with \
                    list_elements / get_element. If link_status says linked, you are editing the \
                    model open in the Ferro app and the user reviews your changes. Every change is one undo step and \
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

/// The tool list as pretty JSON, `{"tools": [...]}` exactly as MCP
/// `tools/list` returns it. Generators (the Python SDK, ADR 0005) read it.
/// Keys are sorted, so the output is deterministic.
fn tools_json() -> String {
    let v = json!({ "tools": tools::list() });
    serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string())
}

fn main() {
    if std::env::args().skip(1).any(|a| a == "--tools-json") {
        println!("{}", tools_json());
        return;
    }
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

    fn server() -> Server {
        let mut s = Server::new();
        s.use_link = false;
        s
    }

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
    fn agent_sets_material_data_and_reads_type_u_value() {
        let mut s = server();
        let r = call(
            &mut s,
            "create_material",
            json!({"name":"Brick","color":"#b5533c","thermal":{"conductivity_w_mk":0.77},"physical":{"density_kg_m3":1800}}),
        );
        assert_eq!(r["is_error"], false, "{r}");
        let brick = r["body"]["result"]["element"].as_u64().unwrap();
        let r = call(
            &mut s,
            "create_material",
            json!({"name":"Wool","color":"#eeee88"}),
        );
        let wool = r["body"]["result"]["element"].as_u64().unwrap();
        let r = call(
            &mut s,
            "create_wall_type",
            json!({"name":"Brick + wool","layers":[
                {"function":"structure","thickness":"200mm","material":brick},
                {"function":"insulation","thickness":"10cm","material":wool}]}),
        );
        let wt = r["body"]["result"]["element"].as_u64().unwrap();

        // Wool lacks data: U-value is null and the layer is named.
        let d = call(&mut s, "get_element", json!({"element": wt}))["body"].clone();
        let q = &d["quantities"];
        assert!(q["u_value_w_m2k"].is_null(), "{d}");
        assert!(q["missing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["layer"] == 1 && g["reason"] == "no_conductivity"));

        let r = call(
            &mut s,
            "set_material",
            json!({"material": wool, "thermal": {"conductivity_w_mk": 0.035, "density_kg_m3": 30}}),
        );
        assert_eq!(r["is_error"], false, "{r}");
        let d = call(&mut s, "get_element", json!({"element": wt}))["body"].clone();
        // R = 0.13 + 0.2/0.77 + 0.1/0.035 + 0.04 = 3.2869 → U = 0.3042
        let u = d["quantities"]["u_value_w_m2k"].as_f64().unwrap();
        assert!((u - 0.3042).abs() < 1e-4, "{d}");
        let m = d["quantities"]["mass_kg_m2"].as_f64().unwrap();
        assert!((m - 363.0).abs() < 1e-6, "{d}");

        // Material detail shows its assets.
        let d = call(&mut s, "get_element", json!({"element": wool}))["body"].clone();
        assert!((d["thermal"]["conductivity_w_mk"].as_f64().unwrap() - 0.035).abs() < 1e-12);

        // A typo is an error, not silently ignored.
        let r = call(
            &mut s,
            "set_material",
            json!({"material": wool, "thermal": {"lambda": 0.04}}),
        );
        assert_eq!(r["is_error"], true, "{r}");
    }

    #[test]
    fn agent_models_a_house_with_units_and_undo() {
        let mut s = server();
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
    fn agent_builds_walls_doors_and_a_floor_in_metres() {
        let mut s = server();
        let r = call(
            &mut s,
            "create_walls",
            json!({"points": [[0,0,0],["8m",0,0],["8m","6m",0],[0,"6m",0]], "closed": true, "height": "3m"}),
        );
        assert_eq!(r["is_error"], false, "{r}");
        let walls = r["body"]["result"]["elements"].as_array().unwrap().clone();
        let r = call(
            &mut s,
            "create_opening",
            json!({"wall": walls[0], "kind": "door", "position": "2m"}),
        );
        assert_eq!(r["is_error"], false, "{r}");
        let r = call(
            &mut s,
            "create_floor",
            json!({"points": [[0,0],["8m",0],["8m","6m"],[0,"6m"]]}),
        );
        assert_eq!(r["is_error"], false, "{r}");
        let r = call(
            &mut s,
            "create_wall_type",
            json!({"name": "Brick", "layers": [{"function": "structure", "thickness": "22cm"}]}),
        );
        assert_eq!(r["is_error"], false, "{r}");
        let l = call(&mut s, "list_elements", json!({"category": "walls"}));
        let first = l["body"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == walls[0])
            .unwrap()
            .clone();
        assert_eq!(first["length_m"], 8.0);
        assert_eq!(first["height_m"], 3.0);
        let d = call(&mut s, "get_element", json!({"element": walls[0]}));
        assert!(d["body"]["faces"].as_array().unwrap().len() > 6);
        let r = call(
            &mut s,
            "create_opening",
            json!({"wall": walls[0], "kind": "window", "position": "7.9m"}),
        );
        assert_eq!(r["is_error"], true);
        assert_eq!(
            r["body"]["failures"][0]["code"],
            "ferro.opening.outside_host"
        );
        let st = call(&mut s, "link_status", json!({}));
        assert_eq!(st["body"]["linked"], false);
    }

    #[test]
    fn failures_are_reported_and_change_nothing() {
        let mut s = server();
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
        let mut s = server();
        call(
            &mut s,
            "create_box",
            json!({"origin":[0,0,0],"width":1000,"depth":1000,"height":1000}),
        );
        let r = call(&mut s, "save_document", json!({"path": path}));
        assert_eq!(r["is_error"], false, "{r}");
        let mut s2 = server();
        let r = call(&mut s2, "open_document", json!({"path": path}));
        assert_eq!(r["body"]["elements"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tools_json_matches_tools_list() {
        let printed: Value = serde_json::from_str(&tools_json()).unwrap();
        let mut s = server();
        let listed = handle(
            &mut s,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .unwrap()["result"]
            .clone();
        assert_eq!(printed, listed);
        assert_eq!(tools_json(), tools_json(), "deterministic");
        assert!(printed["tools"].as_array().unwrap().len() > 40);
    }

    #[test]
    fn python_sdk_snapshot_is_current() {
        // python/ferro/tools.json feeds scripts/gen_python.py (ADR 0005).
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../python/ferro/tools.json");
        let snapshot = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            snapshot.trim_end(),
            tools_json(),
            "python/ferro/tools.json is stale: run `python3 scripts/gen_python.py --refresh`"
        );
    }

    #[test]
    fn notifications_get_no_reply() {
        let mut s = server();
        assert!(handle(
            &mut s,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none());
    }
}
