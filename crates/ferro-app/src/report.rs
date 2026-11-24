//! Problem reports (ADR 0003 §4, M5-12.5).
//!
//! The app keeps a short log of what happened in this session: command
//! names and their outcome codes, status messages, crashes. It never logs
//! document content (no coordinates, names, parameter values). When the app
//! crashes, a report file is written to `~/.ferro/reports/`; on the next
//! start the user is offered to review and send it. Nothing is sent
//! automatically: "Report a problem" opens a prefilled GitHub issue that the
//! user reads before submitting.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Lines kept in the session log.
pub const LOG_LINES: usize = 200;
/// Where problems are reported.
pub const ISSUES_URL: &str = "https://github.com/ktsdatpham/ferro/issues/new";
/// Longest body put in the issue URL (browsers and GitHub limit URL size);
/// the full report stays in the saved file.
const URL_BODY_LIMIT: usize = 6000;

fn log() -> &'static Mutex<VecDeque<String>> {
    static LOG: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    LOG.get_or_init(|| Mutex::new(VecDeque::with_capacity(LOG_LINES)))
}

fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Add a line to the session log (oldest lines drop off).
pub fn record(line: impl Into<String>) {
    if let Ok(mut l) = log().lock() {
        if l.len() >= LOG_LINES {
            l.pop_front();
        }
        l.push_back(format!("{} {}", now_s(), line.into()));
    }
}

/// The session log, oldest first.
pub fn session_log() -> Vec<String> {
    log()
        .lock()
        .map(|l| l.iter().cloned().collect())
        .unwrap_or_default()
}

/// A report: app version, platform and the session log, plus an optional
/// description of what went wrong (crash message).
pub fn report_text(what: &str) -> String {
    let mut s = format!(
        "## What happened\n\n{what}\n\n## Environment\n\n- Ferro {}\n- {} / {}\n\n## Session log (no model data)\n\n```\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    for l in session_log() {
        s.push_str(&l);
        s.push('\n');
    }
    s.push_str("```\n");
    s
}

/// `~/.ferro/reports` (or `$FERRO_HOME/reports`).
pub fn reports_dir() -> Option<PathBuf> {
    ferro_tools::link::ferro_dir().map(|d| d.join("reports"))
}

/// Save a report file in [`reports_dir`]; returns its path.
pub fn save(text: &str, prefix: &str) -> Option<PathBuf> {
    save_in(&reports_dir()?, text, prefix)
}

fn save_in(dir: &std::path::Path, text: &str, prefix: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join(format!("{prefix}-{}.md", now_s()));
    std::fs::write(&path, text).ok()?;
    Some(path)
}

/// Crash reports left by a previous session, oldest first.
pub fn pending_crashes() -> Vec<PathBuf> {
    reports_dir().map(|d| pending_in(&d)).unwrap_or_default()
}

fn pending_in(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".md"))
        })
        .collect();
    v.sort();
    v
}

/// Mark a crash report as handled (renamed so it is not offered again).
pub fn dismiss(path: &std::path::Path) {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.replacen("crash-", "seen-", 1));
    if let Some(n) = name {
        let _ = std::fs::rename(path, path.with_file_name(n));
    }
}

/// Install a panic hook that writes a crash report, then runs the default
/// hook (so the message still reaches the terminal).
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let what = format!("Ferro crashed.\n\n```\n{info}\n\n{bt}\n```");
        let _ = save(&report_text(&what), "crash");
        default(info);
    }));
}

/// Percent-encode for a URL query value.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A prefilled "new issue" URL. Long bodies are cut (at a character
/// boundary) with a note to attach the saved file.
pub fn issue_url(title: &str, body: &str) -> String {
    let mut b = body.to_string();
    if b.len() > URL_BODY_LIMIT {
        let mut cut = URL_BODY_LIMIT;
        while !b.is_char_boundary(cut) {
            cut -= 1;
        }
        b.truncate(cut);
        b.push_str("\n\n(cut: please attach the saved report file)");
    }
    format!("{ISSUES_URL}?title={}&body={}", encode(title), encode(&b))
}

/// Open a URL in the user's browser. No shell parses the URL (on Windows
/// it goes to `rundll32 url.dll,FileProtocolHandler`, not `cmd /C start`,
/// so `&` or `|` in a URL from a server cannot run commands).
pub fn open_url(url: &str) -> Result<(), String> {
    let r = if cfg!(target_os = "windows") {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    r.map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_keeps_the_last_lines_only() {
        for i in 0..(LOG_LINES + 5) {
            record(format!("line {i}"));
        }
        let l = session_log();
        assert_eq!(l.len(), LOG_LINES);
        assert!(l
            .last()
            .is_some_and(|x| x.ends_with(&format!("line {}", LOG_LINES + 4))));
        assert!(report_text("x").contains("## Session log"));
    }

    #[test]
    fn issue_url_is_encoded_and_bounded() {
        let u = issue_url("Crash: é & ?", "a b\n");
        assert!(u.starts_with(ISSUES_URL));
        assert!(u.contains("title=Crash%3A%20%C3%A9%20%26%20%3F"));
        assert!(u.contains("body=a%20b%0A"));
        // Long bodies are cut, even in the middle of a multi-byte character.
        let long = "é".repeat(URL_BODY_LIMIT);
        let u = issue_url("t", &long);
        assert!(u.contains("attach%20the%20saved%20report%20file"));
        assert!(u.len() < URL_BODY_LIMIT * 4);
        // Empty input still makes a valid URL.
        assert!(issue_url("", "").ends_with("?title=&body="));
    }

    #[test]
    fn crash_files_are_found_and_dismissed() {
        let dir = std::env::temp_dir().join(format!("ferro-report-test-{}", now_s()));
        assert!(
            pending_in(&dir).is_empty(),
            "missing folder: nothing pending"
        );
        let p = save_in(&dir, "report", "crash").unwrap();
        save_in(&dir, "manual", "report").unwrap();
        assert_eq!(pending_in(&dir), vec![p.clone()]);
        dismiss(&p);
        assert!(pending_in(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
