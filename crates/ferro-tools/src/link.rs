//! Ferro Link (D-65): lets an agent drive the Ferro app that is open on the
//! same computer. The app listens on `127.0.0.1` (random port) and writes
//! `~/.ferro/link.json` with the port and a secret token; clients send one
//! JSON request per line and read one JSON response per line.
//!
//! Request: `{"token": "...", "tool": "create_wall", "args": {...},
//! "agent": {"name": "claude-code", "version": "1.0"}, "prompt": "..."}`
//! Response: `{"ok": true, "body": {...}}`.
//!
//! Only connections from the local machine are accepted, and every request
//! must carry the token from the link file, which only the user can read.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

/// File name inside the Ferro folder.
pub const LINK_FILE: &str = "link.json";

/// Contents of `link.json`, written by the app while it is open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkInfo {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub project: String,
    pub version: String,
}

/// Who is calling (recorded in the journal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub name: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkRequest {
    pub token: String,
    pub tool: String,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub agent: Option<AgentInfo>,
    #[serde(default)]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkResponse {
    pub ok: bool,
    pub body: Value,
}

/// The per-user Ferro folder: `$FERRO_HOME`, else `~/.ferro`.
pub fn ferro_dir() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("FERRO_HOME") {
        return Some(PathBuf::from(h));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).join(".ferro"))
}

/// Path of `link.json`.
pub fn link_path() -> Option<PathBuf> {
    ferro_dir().map(|d| d.join(LINK_FILE))
}

/// Read `link.json`, if the app has written one.
pub fn read_info() -> Option<LinkInfo> {
    let text = std::fs::read_to_string(link_path()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write `link.json` (app side).
pub fn write_info(info: &LinkInfo) -> io::Result<()> {
    let path = link_path().ok_or_else(|| io::Error::other("no home folder"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(info).map_err(io::Error::other)?;
    std::fs::write(&path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Remove `link.json` if it still belongs to this app instance.
pub fn remove_info(token: &str) {
    if let (Some(path), Some(info)) = (link_path(), read_info()) {
        if info.token == token {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Send one request to the app and wait for its answer.
pub fn send(info: &LinkInfo, req: &LinkRequest, timeout: Duration) -> io::Result<LinkResponse> {
    let addr = SocketAddr::from(([127, 0, 0, 1], info.port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let line = serde_json::to_string(req).map_err(io::Error::other)?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut answer = String::new();
    reader.read_line(&mut answer)?;
    serde_json::from_str(&answer).map_err(io::Error::other)
}

/// Read one request line from a client (app side).
pub fn read_request(stream: &TcpStream) -> io::Result<LinkRequest> {
    // Requests are small; cap the size to keep a stray client harmless.
    let mut reader = BufReader::new(stream.take(4 << 20));
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "empty request",
        ));
    }
    serde_json::from_str(&line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Write one response line (app side).
pub fn write_response(mut stream: &TcpStream, resp: &LinkResponse) -> io::Result<()> {
    let line = serde_json::to_string(resp).map_err(io::Error::other)?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn request_roundtrip_over_tcp() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let req = read_request(&s).unwrap();
            let ok = req.token == "secret";
            write_response(
                &s,
                &LinkResponse {
                    ok,
                    body: serde_json::json!({"echo": req.tool}),
                },
            )
            .unwrap();
        });
        let info = LinkInfo {
            port,
            token: "secret".into(),
            pid: 1,
            project: "P".into(),
            version: "0".into(),
        };
        let r = send(
            &info,
            &LinkRequest {
                token: "secret".into(),
                tool: "list_elements".into(),
                args: Value::Null,
                agent: None,
                prompt: None,
            },
            Duration::from_secs(5),
        )
        .unwrap();
        server.join().unwrap();
        assert!(r.ok);
        assert_eq!(r.body["echo"], "list_elements");
    }

    #[test]
    fn unreachable_port_fails_fast() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let info = LinkInfo {
            port,
            token: "x".into(),
            pid: 1,
            project: String::new(),
            version: String::new(),
        };
        let req = LinkRequest {
            token: "x".into(),
            tool: "t".into(),
            args: Value::Null,
            agent: None,
            prompt: None,
        };
        assert!(send(&info, &req, Duration::from_secs(1)).is_err());
    }
}
