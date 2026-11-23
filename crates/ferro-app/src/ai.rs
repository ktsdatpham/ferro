//! In-app AI chat (D-26, D-44): the user brings their own key for the
//! Anthropic Messages API or any OpenAI-compatible endpoint (OpenAI, Ollama,
//! LM Studio, ...). The model gets the same tools as MCP agents; its tool
//! calls go through Ferro Link to the UI thread, so its changes arrive as a
//! proposal the user accepts or rejects.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

/// Which wire format the endpoint speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Anthropic,
    OpenaiCompatible,
}

/// Chat settings, stored in `~/.ferro/settings.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiSettings {
    pub provider: Provider,
    pub model: String,
    /// Stored in plain text (the UI says so).
    #[serde(default)]
    pub api_key: String,
    /// Endpoint root, e.g. `https://api.anthropic.com` or
    /// `http://localhost:11434/v1`.
    pub base_url: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            provider: Provider::Anthropic,
            model: "claude-sonnet-5-5".into(),
            api_key: String::new(),
            base_url: "https://api.anthropic.com".into(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    ai: Option<AiSettings>,
}

fn settings_path() -> Option<PathBuf> {
    ferro_tools::link::ferro_dir().map(|d| d.join("settings.json"))
}

impl AiSettings {
    pub fn load() -> AiSettings {
        settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<SettingsFile>(&t).ok())
            .and_then(|f| f.ai)
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = settings_path().ok_or("no home folder")?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let f = SettingsFile {
            ai: Some(self.clone()),
        };
        let text = serde_json::to_string_pretty(&f).map_err(|e| e.to_string())?;
        std::fs::write(&p, text).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn is_ready(&self) -> bool {
        !self.model.trim().is_empty()
            && (self.provider == Provider::OpenaiCompatible || !self.api_key.trim().is_empty())
    }
}

/// What the chat panel shows.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    Assistant(String),
    Tool {
        name: String,
        ok: bool,
        summary: String,
    },
    Error(String),
    Done,
}

/// HTTP POST of a JSON body; returns the JSON answer or an error message.
pub type Post<'a> = dyn Fn(&str, &[(String, String)], &Value) -> Result<Value, String> + 'a;
/// Run a tool; Ok / Err carry the JSON tool result.
pub type Exec<'a> = dyn Fn(&str, &Value) -> Result<Value, Value> + 'a;

const SYSTEM: &str = "You are the design assistant inside Ferro, an architectural 3D modelling app. \
Use the tools to inspect and change the model. Units are millimetres unless you write a unit (\"3m\"); \
Z is up. Draw exterior walls clockwise seen from above (the exterior is on the left, as in Revit). \
Check the model with list_elements before editing when you need ids. Your changes are shown to the \
user as a proposal they accept or reject; keep each request focused and explain briefly what you did.";

/// Most model ↔ tool round trips per user message.
const MAX_STEPS: usize = 24;

fn tool_defs(provider: Provider) -> Vec<Value> {
    ferro_tools::list()
        .into_iter()
        .map(|t| match provider {
            Provider::Anthropic => json!({
                "name": t["name"], "description": t["description"], "input_schema": t["inputSchema"]
            }),
            Provider::OpenaiCompatible => json!({
                "type": "function",
                "function": {"name": t["name"], "description": t["description"], "parameters": t["inputSchema"]}
            }),
        })
        .collect()
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 160 {
        format!("{}…", s.chars().take(160).collect::<String>())
    } else {
        s
    }
}

/// One user message: call the model, run its tool calls, repeat until it
/// answers in text. `history` holds provider-native messages and grows.
pub fn run_turn(
    cfg: &AiSettings,
    history: &mut Vec<Value>,
    user_text: &str,
    post: &Post,
    exec: &Exec,
    emit: &mut dyn FnMut(ChatEvent),
) -> Result<(), String> {
    let base = cfg.base_url.trim_end_matches('/');
    match cfg.provider {
        Provider::Anthropic => {
            history.push(json!({"role": "user", "content": user_text}));
            let url = format!("{base}/v1/messages");
            let headers = vec![
                ("x-api-key".to_string(), cfg.api_key.clone()),
                ("anthropic-version".to_string(), "2023-06-01".to_string()),
            ];
            for _ in 0..MAX_STEPS {
                let body = json!({
                    "model": cfg.model, "max_tokens": 4096, "system": SYSTEM,
                    "tools": tool_defs(Provider::Anthropic), "messages": history,
                });
                let r = post(&url, &headers, &body)?;
                if let Some(e) = r.get("error") {
                    return Err(e["message"].as_str().unwrap_or("API error").to_string());
                }
                let content = r["content"].as_array().cloned().unwrap_or_default();
                history.push(json!({"role": "assistant", "content": content}));
                let mut results = Vec::new();
                for block in &content {
                    match block["type"].as_str() {
                        Some("text") => {
                            let t = block["text"].as_str().unwrap_or("").trim();
                            if !t.is_empty() {
                                emit(ChatEvent::Assistant(t.to_string()));
                            }
                        }
                        Some("tool_use") => {
                            let name = block["name"].as_str().unwrap_or("");
                            let (ok, out) = match exec(name, &block["input"]) {
                                Ok(v) => (true, v),
                                Err(v) => (false, v),
                            };
                            emit(ChatEvent::Tool {
                                name: name.to_string(),
                                ok,
                                summary: short(&block["input"]),
                            });
                            results.push(json!({
                                "type": "tool_result", "tool_use_id": block["id"],
                                "content": out.to_string(), "is_error": !ok
                            }));
                        }
                        _ => {}
                    }
                }
                if results.is_empty() {
                    emit(ChatEvent::Done);
                    return Ok(());
                }
                history.push(json!({"role": "user", "content": results}));
            }
        }
        Provider::OpenaiCompatible => {
            if history.is_empty() {
                history.push(json!({"role": "system", "content": SYSTEM}));
            }
            history.push(json!({"role": "user", "content": user_text}));
            let url = format!("{base}/chat/completions");
            let mut headers = Vec::new();
            if !cfg.api_key.trim().is_empty() {
                headers.push((
                    "authorization".to_string(),
                    format!("Bearer {}", cfg.api_key.trim()),
                ));
            }
            for _ in 0..MAX_STEPS {
                let body = json!({
                    "model": cfg.model, "messages": history, "tools": tool_defs(Provider::OpenaiCompatible),
                });
                let r = post(&url, &headers, &body)?;
                if let Some(e) = r.get("error") {
                    return Err(e["message"].as_str().unwrap_or("API error").to_string());
                }
                let msg = r["choices"][0]["message"].clone();
                if msg.is_null() {
                    return Err("the endpoint returned no message".into());
                }
                history.push(msg.clone());
                if let Some(t) = msg["content"].as_str().map(str::trim) {
                    if !t.is_empty() {
                        emit(ChatEvent::Assistant(t.to_string()));
                    }
                }
                let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
                if calls.is_empty() {
                    emit(ChatEvent::Done);
                    return Ok(());
                }
                for c in calls {
                    let name = c["function"]["name"].as_str().unwrap_or("");
                    let args: Value = match &c["function"]["arguments"] {
                        Value::String(s) => serde_json::from_str(s).unwrap_or(json!({})),
                        v => v.clone(),
                    };
                    let (ok, out) = match exec(name, &args) {
                        Ok(v) => (true, v),
                        Err(v) => (false, v),
                    };
                    emit(ChatEvent::Tool {
                        name: name.to_string(),
                        ok,
                        summary: short(&args),
                    });
                    history.push(json!({"role": "tool", "tool_call_id": c["id"], "content": out.to_string()}));
                }
            }
        }
    }
    Err(format!("stopped after {MAX_STEPS} steps"))
}

/// Real HTTP transport (blocking; call from a worker thread).
pub fn http_post(url: &str, headers: &[(String, String)], body: &Value) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(180)))
        .build()
        .into();
    let mut req = agent.post(url).header("content-type", "application/json");
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let mut resp = req.send_json(body).map_err(|e| e.to_string())?;
    let status = resp.status();
    let v: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("HTTP {status}: {e}"))?;
    if !status.is_success() && v.get("error").is_none() {
        return Err(format!("HTTP {status}: {v}"));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn anthropic_tool_loop_runs_tools_and_finishes() {
        let cfg = AiSettings {
            api_key: "k".into(),
            ..Default::default()
        };
        let calls = RefCell::new(0);
        let post = |url: &str, headers: &[(String, String)], body: &Value| {
            assert!(url.ends_with("/v1/messages"));
            assert!(headers.iter().any(|(k, v)| k == "x-api-key" && v == "k"));
            assert!(body["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == "create_wall"));
            let mut n = calls.borrow_mut();
            *n += 1;
            Ok(if *n == 1 {
                json!({"content": [
                    {"type": "text", "text": "Adding a wall."},
                    {"type": "tool_use", "id": "t1", "name": "create_wall", "input": {"start": [0,0,0], "end": ["4m",0,0]}}
                ]})
            } else {
                let last = body["messages"].as_array().unwrap().last().unwrap().clone();
                assert_eq!(last["content"][0]["type"], "tool_result");
                assert_eq!(last["content"][0]["tool_use_id"], "t1");
                json!({"content": [{"type": "text", "text": "Done."}]})
            })
        };
        let exec = |name: &str, _args: &Value| {
            assert_eq!(name, "create_wall");
            Ok(json!({"result": {"element": 3}}))
        };
        let mut events = Vec::new();
        let mut history = Vec::new();
        run_turn(&cfg, &mut history, "draw a wall", &post, &exec, &mut |e| {
            events.push(e)
        })
        .unwrap();
        assert_eq!(*calls.borrow(), 2);
        assert_eq!(events[0], ChatEvent::Assistant("Adding a wall.".into()));
        assert!(
            matches!(&events[1], ChatEvent::Tool { name, ok: true, .. } if name == "create_wall")
        );
        assert_eq!(events.last(), Some(&ChatEvent::Done));
        assert_eq!(history.len(), 4);
    }

    #[test]
    fn openai_compatible_parses_string_arguments() {
        let cfg = AiSettings {
            provider: Provider::OpenaiCompatible,
            model: "llama".into(),
            api_key: String::new(),
            base_url: "http://localhost:11434/v1/".into(),
        };
        let calls = RefCell::new(0);
        let post = |url: &str, headers: &[(String, String)], body: &Value| {
            assert_eq!(url, "http://localhost:11434/v1/chat/completions");
            assert!(headers.is_empty());
            assert_eq!(body["messages"][0]["role"], "system");
            let mut n = calls.borrow_mut();
            *n += 1;
            Ok(if *n == 1 {
                json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "list_elements", "arguments": "{\"category\":\"walls\"}"}}
                ]}}]})
            } else {
                json!({"choices": [{"message": {"role": "assistant", "content": "There are no walls."}}]})
            })
        };
        let exec = |name: &str, args: &Value| {
            assert_eq!(name, "list_elements");
            assert_eq!(args["category"], "walls");
            Err(json!({"error": "nope"}))
        };
        let mut events = Vec::new();
        let mut history = Vec::new();
        run_turn(&cfg, &mut history, "walls?", &post, &exec, &mut |e| {
            events.push(e)
        })
        .unwrap();
        assert!(matches!(&events[0], ChatEvent::Tool { ok: false, .. }));
        assert_eq!(
            events[1],
            ChatEvent::Assistant("There are no walls.".into())
        );
        assert_eq!(history[3]["role"], "tool");
    }

    #[test]
    fn api_errors_surface() {
        let cfg = AiSettings {
            api_key: "bad".into(),
            ..Default::default()
        };
        let post = |_: &str, _: &[(String, String)], _: &Value| {
            Ok(
                json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}}),
            )
        };
        let exec = |_: &str, _: &Value| Ok(json!({}));
        let r = run_turn(&cfg, &mut Vec::new(), "hi", &post, &exec, &mut |_| {});
        assert_eq!(r.unwrap_err(), "invalid x-api-key");
    }

    #[test]
    fn readiness_needs_a_key_for_anthropic_only() {
        let mut s = AiSettings::default();
        assert!(!s.is_ready());
        s.api_key = "x".into();
        assert!(s.is_ready());
        let local = AiSettings {
            provider: Provider::OpenaiCompatible,
            model: "m".into(),
            api_key: String::new(),
            base_url: "http://localhost:1234/v1".into(),
        };
        assert!(local.is_ready());
    }
}
