//! The model corpus (M5-17): every script in `tests/corpus/` is rebuilt,
//! checked (`ferro-cli check`) and compared with its blessed fingerprint
//! (`<name>.fingerprint.json`: face ids, counts, areas, volumes, bounding
//! boxes, relationships). A kernel change that renames faces, loses lineage
//! or changes a quantity fails here.
//!
//! After an intended change, re-bless and review the diff:
//! `FERRO_BLESS=1 cargo test -p ferro-cli --test corpus`.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_ferro-cli");

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

fn cli(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .env("FERRO_LINK", "off")
        .output()
        .expect("ferro-cli runs")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn scripts() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(corpus())
        .expect("corpus dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && !p.to_string_lossy().ends_with(".fingerprint.json")
        })
        .collect();
    v.sort();
    v
}

/// Build `script` into a model in a temporary dir; returns the model path.
fn build(script: &Path, dir: &Path) -> PathBuf {
    let stem = script.file_stem().unwrap().to_string_lossy();
    let model = dir.join(format!("{stem}.fro"));
    let o = cli(&[
        "run",
        script.to_str().unwrap(),
        "--out",
        model.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{stem}: run failed\n{}", out(&o));
    let o = cli(&["check", model.to_str().unwrap()]);
    assert!(o.status.success(), "{stem}: check failed\n{}", out(&o));
    model
}

fn tmp(test: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ferro-corpus-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn every_corpus_model_matches_its_fingerprint() {
    let dir = tmp("all");
    let bless = std::env::var_os("FERRO_BLESS").is_some();
    let list = scripts();
    assert!(list.len() >= 7, "corpus has {} scripts", list.len());
    let mut failures = Vec::new();
    for script in list {
        let model = build(&script, &dir);
        let expected = script.with_extension("fingerprint.json");
        if bless || !expected.exists() {
            let o = cli(&["fingerprint", model.to_str().unwrap()]);
            assert!(o.status.success(), "{}", out(&o));
            std::fs::write(&expected, o.stdout).unwrap();
            if !bless {
                failures.push(format!(
                    "{}: no fingerprint yet, wrote one; review and commit it",
                    expected.display()
                ));
            }
            continue;
        }
        let o = cli(&[
            "fingerprint",
            model.to_str().unwrap(),
            "--expect",
            expected.to_str().unwrap(),
        ]);
        if !o.status.success() {
            failures.push(format!("{}:\n{}", script.display(), out(&o)));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        failures.is_empty(),
        "{}\n(after an intended change: FERRO_BLESS=1 cargo test -p ferro-cli --test corpus)",
        failures.join("\n")
    );
}

fn fingerprint(script: &str, dir: &Path) -> Value {
    let model = build(&corpus().join(script), dir);
    let o = cli(&["fingerprint", model.to_str().unwrap()]);
    serde_json::from_slice(&o.stdout).expect("json")
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

/// Hand-computed oracles, independent of the blessed files.
#[test]
fn oracle_quantities() {
    let dir = tmp("oracle");
    let f = fingerprint("push-pull.json", &dir);
    let vol = |id: &str| f["elements"][id]["volume"].as_f64().unwrap_or(f64::NAN);
    // 2 m cube minus a 1 × 1 × 0.5 m pocket.
    assert!(close(vol("1"), 7.5e9), "pocket {}", vol("1"));
    // 2 × 1 × 1 m box pulled up 1.5 m (rotation keeps volume).
    assert!(close(vol("2"), 5.0e9), "boss {}", vol("2"));
    // Copies keep the volume and the face ids of the original.
    for c in ["3", "4"] {
        assert!(close(vol(c), 7.5e9));
        assert_eq!(f["elements"][c]["faces"], f["elements"]["1"]["faces"]);
    }
    // The pocket floor is 1 m² facing up, 500 mm below the top.
    let faces = f["elements"]["1"]["faces"].as_array().unwrap();
    let up_1m2 = faces
        .iter()
        .filter(|x| {
            x["normal"] == serde_json::json!([0.0, 0.0, 1.0])
                && close(x["area"].as_f64().unwrap(), 1e6)
        })
        .count();
    assert_eq!(up_1m2, 1, "{faces:?}");
    // The top kept its id (5) through imprint and push: lineage.
    assert!(faces
        .iter()
        .any(|x| x["face"] == 5 && close(x["area"].as_f64().unwrap(), 3e6)));

    let f = fingerprint("relationships.json", &dir);
    // Deleting box 3 removed the aggregate and the connect rows; the join stays.
    assert_eq!(
        f["elements"]["1"]["relationships"],
        serde_json::json!([{"kind": "join", "elements": [1, 2]}])
    );
    assert!(f["elements"].get("3").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
