//! Runs the `ferro-cli` binary on the scripts in `tests/scripts/`.

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_ferro-cli");

fn script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts")
        .join(name)
}

/// A fresh temporary directory per test.
fn tmp(test: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ferro-cli-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn cli(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .env("FERRO_LINK", "off")
        .output()
        .unwrap()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

fn lines(o: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

fn json(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// Build the house into `dir/house.fro`.
fn house(dir: &Path) -> PathBuf {
    let fro = dir.join("house.fro");
    let o = cli(&["run", s(&script("house.json")), "--out", s(&fro)]);
    assert_eq!(code(&o), 0, "{}", String::from_utf8_lossy(&o.stdout));
    fro
}

#[test]
fn house_script_writes_fro_and_ifc_and_passes_check() {
    let d = tmp("house");
    let fro = d.join("house.fro");
    let ifc = d.join("house.ifc");
    let o = cli(&[
        "run",
        s(&script("house.json")),
        "--out",
        s(&fro),
        "--ifc",
        s(&ifc),
        "--obj",
        s(&d.join("house.obj")),
        "--glb",
        s(&d.join("house.glb")),
    ]);
    assert_eq!(code(&o), 0, "{}", String::from_utf8_lossy(&o.stdout));
    let l = lines(&o);
    // 5 calls + 4 outputs + summary.
    assert_eq!(l.len(), 10);
    assert!(l[..9].iter().all(|x| x["ok"] == true));
    assert_eq!(l[0]["tool"], "create_level");
    assert_eq!(l[3]["step"], 3);
    assert_eq!(l[9]["summary"]["ok"], true);
    assert_eq!(l[9]["summary"]["elements"], 10);
    let ifc_text = std::fs::read_to_string(&ifc).unwrap();
    for class in [
        "IFCWALL",
        "IFCSLAB",
        "IFCDOOR",
        "IFCWINDOW",
        "IFCBUILDINGSTOREY",
    ] {
        assert!(ifc_text.contains(class), "IFC has no {class}");
    }

    let o = cli(&["check", s(&fro)]);
    assert_eq!(code(&o), 0, "{}", String::from_utf8_lossy(&o.stdout));
    let r = json(&o);
    assert_eq!(r["ok"], true);
    let names: Vec<_> = r["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["check"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "open",
            "regenerate",
            "bodies",
            "roundtrip_json",
            "roundtrip_fro",
            "deterministic_export"
        ]
    );
    assert_eq!(r["checks"][2]["validated"], 7);

    // The same check passes on the JSON interchange file.
    let j = d.join("house.fro.json");
    let o = cli(&["call", "list_elements", "--in", s(&fro), "--out", s(&j)]);
    assert_eq!(code(&o), 0);
    assert_eq!(code(&cli(&["check", s(&j)])), 0);
}

#[test]
fn failing_script_exits_nonzero_with_the_failure_code() {
    let o = cli(&["run", s(&script("failing.json"))]);
    assert_eq!(code(&o), 1);
    let l = lines(&o);
    // Stops at the first failure: walls, window, summary.
    assert_eq!(l.len(), 3);
    assert_eq!(l[1]["ok"], false);
    assert_eq!(l[1]["failures"][0]["code"], "ferro.opening.outside_host");
    assert_eq!(l[2]["summary"]["failed"], 1);

    let o = cli(&["run", s(&script("failing.json")), "--keep-going"]);
    assert_eq!(code(&o), 1);
    let l = lines(&o);
    assert_eq!(l.len(), 4);
    assert_eq!(l[2]["ok"], true, "keeps going after the failure");

    // Bad units are a tool error with a code too.
    let o = cli(&[
        "call",
        "create_box",
        r#"{"origin":[0,0,0],"width":"abc","depth":1,"height":1}"#,
    ]);
    assert_eq!(code(&o), 1);
    assert_eq!(lines(&o)[0]["failures"][0]["code"], "ferro.cli.tool_error");
}

#[test]
fn atomic_leaves_nothing_behind_on_failure() {
    let d = tmp("atomic");
    let fro = house(&d);
    let before = json(&cli(&["info", s(&fro)]));

    let after = d.join("after.fro");
    let o = cli(&[
        "run",
        s(&script("failing.json")),
        "--in",
        s(&fro),
        "--atomic",
        "--keep-going",
        "--out",
        s(&after),
    ]);
    assert_eq!(code(&o), 1);
    let l = lines(&o);
    let summary = &l[l.len() - 1]["summary"];
    assert_eq!(summary["rolled_back"], true);
    assert_eq!(summary["elements"], 10);
    assert_eq!(json(&cli(&["info", s(&after)])), before);

    // Without --atomic the successful calls stay.
    let o = cli(&[
        "run",
        s(&script("failing.json")),
        "--in",
        s(&fro),
        "--keep-going",
    ]);
    let l = lines(&o);
    assert_eq!(l[l.len() - 1]["summary"]["elements"], 10 + 4 + 1);

    // A clean atomic run commits.
    let o = cli(&["run", s(&script("house.json")), "--atomic"]);
    assert_eq!(code(&o), 0);
    assert_eq!(lines(&o)[5]["summary"]["elements"], 10);
}

#[test]
fn bad_input_gives_a_clean_error_never_a_panic() {
    let d = tmp("bad");
    let bad = d.join("bad.json");
    let cases: Vec<(Vec<String>, &str)> = {
        std::fs::write(&bad, "[{\"tool\": ").unwrap();
        let shape = d.join("shape.json");
        std::fs::write(&shape, r#"{"tool": "undo"}"#).unwrap();
        let notdoc = d.join("x.fro.json");
        std::fs::write(&notdoc, "not json").unwrap();
        let v = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        vec![
            (v(&["run", s(&bad)]), "ferro.cli.script"),
            (v(&["run", s(&shape)]), "ferro.cli.script"),
            (v(&["run", s(&d.join("missing.json"))]), "ferro.io.file"),
            (v(&["call", "list_elements", "{"]), "ferro.cli.script"),
            (v(&["call", "list_elements", "[]"]), "ferro.cli.script"),
            (v(&["info", s(&d.join("missing.fro"))]), "ferro.io.file"),
            (v(&["info", s(&notdoc)]), "ferro.io.json"),
            (
                v(&["run", s(&script("house.json")), "--in", s(&notdoc)]),
                "ferro.io.json",
            ),
            (v(&["frobnicate"]), "ferro.cli.usage"),
            (v(&[]), "ferro.cli.usage"),
            (v(&["run", s(&bad), "--wat"]), "ferro.cli.usage"),
            (v(&["run"]), "ferro.cli.usage"),
            (v(&["bench", s(&bad), "--repeat", "x"]), "ferro.cli.usage"),
            (v(&["info", "a.fro", "--out", "b.fro"]), "ferro.cli.usage"),
        ]
    };
    for (args, expected) in cases {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let o = cli(&args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(!err.contains("panicked"), "{args:?}: {err}");
        assert_eq!(
            code(&o),
            2,
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stdout)
        );
        let v = lines(&o);
        assert_eq!(v[0]["ok"], false);
        assert_eq!(v[0]["error"]["code"], expected, "{args:?}");
    }

    // check reports an unreadable file as a failed check (exit 1).
    let o = cli(&["check", s(&d.join("missing.fro"))]);
    assert_eq!(code(&o), 1);
    assert_eq!(
        json(&o)["checks"][0]["problems"][0]["code"],
        "ferro.io.file"
    );
}

#[test]
fn check_finds_a_broken_body() {
    let d = tmp("broken");
    let j = d.join("box.fro.json");
    let o = cli(&[
        "call",
        "create_box",
        r#"{"origin":[0,0,0],"width":"1m","depth":"1m","height":"1m"}"#,
        "--out",
        s(&j),
    ]);
    assert_eq!(code(&o), 0);
    assert_eq!(code(&cli(&["check", s(&j)])), 0);
    // Move one vertex down: its faces are no longer planar.
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&j).unwrap()).unwrap();
    let vertices = doc["elements"][0]["body"]["body"]["vertices"]
        .as_object_mut()
        .unwrap();
    let first = vertices.keys().next().unwrap().clone();
    vertices[&first]["point"]["z"] = serde_json::json!(-500.0);
    std::fs::write(&j, doc.to_string()).unwrap();
    let o = cli(&["check", s(&j)]);
    assert_eq!(code(&o), 1);
    let r = json(&o);
    assert_eq!(r["checks"][2]["check"], "bodies");
    assert_eq!(
        r["checks"][2]["problems"][0]["code"],
        "ferro.topo.non_planar_face"
    );
    let info = json(&cli(&["info", s(&j)]));
    assert_eq!(info["problems"][0]["code"], "ferro.topo.non_planar_face");
}

#[test]
fn info_output_is_stable() {
    let d = tmp("info");
    let fro = house(&d);
    let o = cli(&["info", s(&fro)]);
    assert_eq!(code(&o), 0);
    let expected = std::fs::read_to_string(script("house.info.json"))
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).trim_end(),
        expected.trim_end()
    );
    // Byte-identical when run again on a model built again.
    let d2 = tmp("info2");
    let o2 = cli(&["info", s(&house(&d2))]);
    assert_eq!(o.stdout, o2.stdout);
}

#[test]
fn tools_lists_the_mcp_tools() {
    let o = cli(&["tools", "--names"]);
    assert_eq!(code(&o), 0);
    let text = String::from_utf8_lossy(&o.stdout);
    let names: Vec<&str> = text.lines().collect();
    for t in [
        "create_walls",
        "create_opening",
        "save_document",
        "export_ifc",
    ] {
        assert!(names.contains(&t), "{t}");
    }
    let full = json(&cli(&["tools"]));
    assert_eq!(full["tools"].as_array().unwrap().len(), names.len());
}

#[test]
fn script_from_stdin_and_bench() {
    let mut child = Command::new(BIN)
        .args(["run", "-"])
        .env("FERRO_LINK", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"[{"tool":"create_level","args":{"name":"L","elevation":"3m"}}]"#)
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(code(&o), 0);
    assert_eq!(lines(&o)[0]["ok"], true);

    let o = cli(&["bench", s(&script("house.json")), "--repeat", "3"]);
    assert_eq!(code(&o), 0);
    let r = json(&o);
    assert_eq!(r["repeat"], 3);
    let calls = r["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 5);
    assert!(calls.iter().all(|c| c["median_ms"].as_f64().unwrap() >= 0.0
        && c["max_ms"].as_f64().unwrap() >= c["median_ms"].as_f64().unwrap()));

    let o = cli(&["bench", s(&script("failing.json")), "--repeat", "2"]);
    assert_eq!(code(&o), 1);
}
