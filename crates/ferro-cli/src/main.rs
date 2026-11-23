//! ferro-cli: Ferro without the UI, for agents, CI and benchmarks.
//!
//! Every call goes through the same tool host as MCP (`ferro_mcp::tools::Server`
//! in headless mode), so a script is exactly what an agent would send (D-17,
//! D-44). Output is JSON on stdout, deterministic for the same input apart
//! from ids the model generates (`uid`) and timings (`bench`).
//!
//! ```text
//! ferro-cli run <script.json|-> [--in F] [--out F] [--ifc F] [--obj F] [--glb F] [--keep-going] [--atomic]
//! ferro-cli call <tool> ['<json args>'] [--in F] [--out F] [--ifc F] [--obj F] [--glb F]
//! ferro-cli tools [--names]
//! ferro-cli info <model.fro|model.fro.json>
//! ferro-cli check <model.fro|model.fro.json>
//! ferro-cli fingerprint <model.fro|model.fro.json> [--expect F]
//! ferro-cli bench <script.json|-> [--repeat N] [--in F]
//! ```
//!
//! Exit codes: 0 success; 1 a tool call failed or `check` found a problem;
//! 2 the command line, a script or a file could not be read (`ferro.cli.*`
//! or `ferro.io.*` code on stdout).

/// Print a line on stdout. A closed pipe (`ferro-cli … | head`) ends the
/// program quietly instead of panicking.
#[macro_export]
macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let mut o = std::io::stdout().lock();
        if writeln!(o, $($arg)*).is_err() {
            std::process::exit(0);
        }
    }};
}

mod bench;
mod check;
mod fingerprint;
mod script;

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::ExitCode;

/// Failure codes introduced by the CLI (documented in the crate README).
pub mod codes {
    /// The command line is wrong (unknown command or option, missing value).
    pub const USAGE: &str = "ferro.cli.usage";
    /// A script or argument JSON cannot be read or has the wrong shape.
    pub const SCRIPT: &str = "ferro.cli.script";
    /// A `{"$ref": ...}` argument names no earlier successful step or path.
    pub const BAD_REF: &str = "ferro.cli.bad_ref";
    /// A tool reported an error without a kernel failure code (bad units,
    /// missing argument, unknown tool).
    pub const TOOL_ERROR: &str = "ferro.cli.tool_error";
    /// A saved element's geometry differs from what regeneration builds.
    pub const STALE_GEOMETRY: &str = "ferro.cli.stale_geometry";
    /// Saving and reopening (JSON or `.fro`) changes the document.
    pub const ROUNDTRIP: &str = "ferro.cli.roundtrip";
    /// Two exports of the same document differ.
    pub const NONDETERMINISTIC: &str = "ferro.cli.nondeterministic";
}

/// An error that stops the CLI before or outside tool calls (exit code 2).
#[derive(Debug)]
pub struct CliError {
    /// Stable code (`ferro.cli.*` or `ferro.io.*`).
    pub code: String,
    /// Human message.
    pub message: String,
}

impl CliError {
    /// New error with a code and message.
    pub fn new(code: &str, message: impl Into<String>) -> CliError {
        CliError {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Usage error.
    pub fn usage(message: impl Into<String>) -> CliError {
        CliError::new(codes::USAGE, message)
    }
}

/// Model files written after a successful run (`--out`, `--ifc`, `--obj`,
/// `--glb`) and the model opened before it (`--in`).
#[derive(Debug, Default, Clone)]
pub struct Io {
    /// `--in`: `.fro` or `.fro.json` opened before the first call.
    pub input: Option<PathBuf>,
    /// `--out`: `.fro` or `.fro.json` saved after the last call.
    pub out: Option<PathBuf>,
    /// `--ifc`: IFC4 export path.
    pub ifc: Option<PathBuf>,
    /// `--obj`: Wavefront OBJ export path.
    pub obj: Option<PathBuf>,
    /// `--glb`: binary glTF export path.
    pub glb: Option<PathBuf>,
}

impl Io {
    /// The output tool calls, in a fixed order: save, IFC, OBJ, GLB.
    pub fn output_calls(&self) -> Vec<(&'static str, Value)> {
        let mut v = Vec::new();
        let pairs = [
            ("save_document", &self.out),
            ("export_ifc", &self.ifc),
            ("export_obj", &self.obj),
            ("export_glb", &self.glb),
        ];
        for (tool, p) in pairs {
            if let Some(p) = p {
                v.push((tool, json!({"path": p})));
            }
        }
        v
    }
}

/// Parsed command line options shared by the subcommands.
#[derive(Debug, Default)]
struct Opts {
    positional: Vec<String>,
    io: Io,
    keep_going: bool,
    atomic: bool,
    names: bool,
    repeat: Option<usize>,
    expect: Option<PathBuf>,
}

fn parse_opts(args: &[String]) -> Result<Opts, CliError> {
    let mut o = Opts::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| CliError::usage(format!("{flag} needs a value")))
        };
        match a.as_str() {
            "--in" => o.io.input = Some(value(a)?.into()),
            "--out" => o.io.out = Some(value(a)?.into()),
            "--ifc" => o.io.ifc = Some(value(a)?.into()),
            "--obj" => o.io.obj = Some(value(a)?.into()),
            "--glb" => o.io.glb = Some(value(a)?.into()),
            "--repeat" => {
                let v = value(a)?;
                let n = v.parse::<usize>().ok().filter(|n| *n > 0).ok_or_else(|| {
                    CliError::usage(format!("--repeat needs a positive whole number, got {v:?}"))
                })?;
                o.repeat = Some(n);
            }
            "--expect" => o.expect = Some(value(a)?.into()),
            "--keep-going" => o.keep_going = true,
            "--atomic" => o.atomic = true,
            "--names" => o.names = true,
            "-" => o.positional.push(a.clone()),
            s if s.starts_with("--") => return Err(CliError::usage(format!("unknown option {s}"))),
            _ => o.positional.push(a.clone()),
        }
    }
    Ok(o)
}

const USAGE: &str = "usage:
  ferro-cli run <script.json|-> [--in F] [--out F] [--ifc F] [--obj F] [--glb F] [--keep-going] [--atomic]
  ferro-cli call <tool> ['<json args>'] [--in F] [--out F] [--ifc F] [--obj F] [--glb F]
  ferro-cli tools [--names]
  ferro-cli info <model.fro|model.fro.json>
  ferro-cli check <model.fro|model.fro.json>
  ferro-cli fingerprint <model.fro|model.fro.json> [--expect F]
  ferro-cli bench <script.json|-> [--repeat N] [--in F]";

/// Print `{"ok": false, "error": {code, message}}` and return exit code 2.
fn report(e: &CliError) -> ExitCode {
    crate::outln!(
        "{}",
        json!({"ok": false, "error": {"code": e.code, "message": e.message}})
    );
    eprintln!("ferro-cli: {} ({})", e.message, e.code);
    ExitCode::from(2)
}

fn one_positional(o: &Opts, what: &str) -> Result<String, CliError> {
    match o.positional.as_slice() {
        [p] => Ok(p.clone()),
        [] => Err(CliError::usage(format!("missing {what}"))),
        _ => Err(CliError::usage(format!(
            "expected one {what}, got {}",
            o.positional.len()
        ))),
    }
}

fn run(args: &[String]) -> Result<ExitCode, CliError> {
    let Some((cmd, rest)) = args.split_first() else {
        return Err(CliError::usage(USAGE));
    };
    let o = parse_opts(rest)?;
    let only_io_in = |o: &Opts| {
        if o.io.out.is_some() || o.io.ifc.is_some() || o.io.obj.is_some() || o.io.glb.is_some() {
            Err(CliError::usage(format!("{cmd} does not write files")))
        } else {
            Ok(())
        }
    };
    match cmd.as_str() {
        "run" => {
            let path = one_positional(&o, "script")?;
            let steps = script::parse(&script::read_source(&path)?)?;
            let opts = script::RunOpts {
                keep_going: o.keep_going,
                atomic: o.atomic,
            };
            Ok(script::run(&steps, &o.io, &opts))
        }
        "call" => {
            let (tool, args) = match o.positional.as_slice() {
                [t] => (t.clone(), json!({})),
                [t, a] => (t.clone(), script::parse_args(a)?),
                [] => return Err(CliError::usage("missing tool name")),
                _ => return Err(CliError::usage("expected: call <tool> ['<json args>']")),
            };
            Ok(script::call_one(&tool, args, &o.io))
        }
        "tools" => {
            if !o.positional.is_empty() {
                return Err(CliError::usage("tools takes no arguments"));
            }
            if o.names {
                for t in ferro_mcp::tools::list() {
                    if let Some(n) = t["name"].as_str() {
                        crate::outln!("{n}");
                    }
                }
            } else {
                crate::outln!("{}", ferro_mcp::tools::tools_json());
            }
            Ok(ExitCode::SUCCESS)
        }
        "info" => {
            only_io_in(&o)?;
            let path = one_positional(&o, "model file")?;
            let server = script::open(&PathBuf::from(&path))?;
            let info = check::info(server.session.doc());
            crate::outln!("{}", pretty(&info));
            Ok(ExitCode::SUCCESS)
        }
        "check" => {
            only_io_in(&o)?;
            let path = one_positional(&o, "model file")?;
            let (report, ok) = check::check(&PathBuf::from(&path));
            crate::outln!("{}", pretty(&report));
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        "fingerprint" => {
            only_io_in(&o)?;
            let path = one_positional(&o, "model file")?;
            let server = script::open(&PathBuf::from(&path))?;
            let got = fingerprint::fingerprint(server.session.doc());
            let Some(exp) = &o.expect else {
                crate::outln!("{}", pretty(&got));
                return Ok(ExitCode::SUCCESS);
            };
            let text = std::fs::read_to_string(exp).map_err(|e| CliError {
                code: codes::SCRIPT.into(),
                message: format!("{}: {e}", exp.display()),
            })?;
            let want: Value = serde_json::from_str(&text).map_err(|e| CliError {
                code: codes::SCRIPT.into(),
                message: format!("{}: {e}", exp.display()),
            })?;
            let diff = fingerprint::differences(&want, &got, fingerprint::TOLERANCE);
            crate::outln!("{}", json!({"ok": diff.is_empty(), "differences": diff}));
            Ok(if diff.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        "bench" => {
            only_io_in(&o)?;
            let path = one_positional(&o, "script")?;
            let steps = script::parse(&script::read_source(&path)?)?;
            bench::bench(&steps, o.io.input.as_deref(), o.repeat.unwrap_or(10))
        }
        "help" | "--help" | "-h" => {
            crate::outln!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(CliError::usage(format!(
            "unknown command {other:?}\n{USAGE}"
        ))),
    }
}

/// Pretty JSON (sorted keys, so deterministic).
pub fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(e) => report(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn options_parse_and_reject_unknown_flags() {
        let o = parse_opts(&s(&[
            "a.json", "--in", "x.fro", "--atomic", "--repeat", "3",
        ]))
        .unwrap();
        assert_eq!(o.positional, vec!["a.json"]);
        assert_eq!(o.io.input, Some(PathBuf::from("x.fro")));
        assert!(o.atomic && !o.keep_going);
        assert_eq!(o.repeat, Some(3));
        assert_eq!(parse_opts(&s(&["--bogus"])).unwrap_err().code, codes::USAGE);
        assert_eq!(parse_opts(&s(&["--out"])).unwrap_err().code, codes::USAGE);
        assert_eq!(
            parse_opts(&s(&["--repeat", "0"])).unwrap_err().code,
            codes::USAGE
        );
    }

    #[test]
    fn outputs_are_written_in_a_fixed_order() {
        let io = Io {
            glb: Some("m.glb".into()),
            out: Some("m.fro".into()),
            ..Io::default()
        };
        let names: Vec<_> = io.output_calls().into_iter().map(|(t, _)| t).collect();
        assert_eq!(names, vec!["save_document", "export_glb"]);
    }
}
