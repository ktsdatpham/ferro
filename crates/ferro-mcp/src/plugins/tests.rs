use super::*;
use crate::python::{interpreter_works, RunControl};
use crate::tools::Server;
use std::io::Write as _;

/// A fresh empty folder for one test.
fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "ferro-plugins-{tag}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, file: &str, text: &str) {
    let p = dir.join(file);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

const GOOD: &str = r#"{
  "id": "box-tools",
  "name": "Box tools",
  "version": "0.1.0",
  "author": "TaD",
  "description": "Boxes.",
  "commands": [
    {"id": "box", "label": "Box at origin", "key": "bx", "run": "main:box"},
    {"id": "tidy", "label": "Tidy", "group": "menu", "run": "scripts/tidy.py", "timeout_s": 60}
  ]
}"#;

fn good_plugin(root: &Path, folder: &str) -> PathBuf {
    let d = root.join(folder);
    write(&d, MANIFEST_FILE, GOOD);
    write(&d, "main.py", "def box(doc):\n    pass\n");
    write(&d, "scripts/tidy.py", "result = 1\n");
    d
}

#[test]
fn a_good_manifest_parses() {
    let root = temp("good");
    let d = good_plugin(&root, "box-tools");
    let m = PluginManifest::read(&d).unwrap();
    assert_eq!(m.id, "box-tools");
    assert_eq!(m.author, "TaD");
    assert_eq!(m.commands.len(), 2);
    let c = &m.commands[0];
    assert_eq!(c.key.as_deref(), Some("BX"), "keys are upper-cased");
    assert_eq!(c.group, PluginGroup::Panel, "panel by default");
    assert_eq!(
        c.entry,
        PluginEntry::Function {
            module: "main".into(),
            function: "box".into()
        }
    );
    let t = &m.commands[1];
    assert_eq!(t.group, PluginGroup::Menu);
    assert_eq!(
        t.entry,
        PluginEntry::Script(PathBuf::from("scripts/tidy.py"))
    );
    assert_eq!(t.timeout_s, Some(60.0));
    // Written back and read again: the same manifest.
    assert_eq!(PluginManifest::parse(&m.to_json(), Some(&d)).unwrap(), m);
    // No marketplace fields: a free, locally made plugin that needs nothing.
    assert_eq!((m.publisher.as_ref(), m.price.as_ref()), (None, None));
    assert!(!m.is_paid());
    assert_eq!(m.required_entitlement(), None);
    assert!(check_entitlement(&m, |_| false).is_ok());
    let _ = std::fs::remove_dir_all(root);
}

fn with_market(fields: &str) -> Result<PluginManifest, String> {
    PluginManifest::parse(
        &format!(
            r#"{{"id": "stairs", "name": "Stairs", "version": "1", "author": "a", {fields}
                "commands": [{{"id": "go", "label": "Go", "run": "main:go"}}]}}"#
        ),
        None,
    )
}

#[test]
fn marketplace_fields_parse_and_paid_commands_need_their_entitlement() {
    let m = with_market(r#""publisher": "acme", "price": {"kind": "one_time", "amount": 490000},"#)
        .unwrap();
    assert_eq!(m.publisher.as_deref(), Some("acme"));
    assert!(m.is_paid());
    assert_eq!(m.required_entitlement().as_deref(), Some("plugin:stairs"));
    let e = check_entitlement(&m, |_| false).unwrap_err();
    assert_eq!(e["failures"][0]["code"], LICENSE_MISSING);
    assert_eq!(e["failures"][0]["plugin"], "stairs");
    assert_eq!(e["failures"][0]["entitlement"], "plugin:stairs");
    assert!(check_entitlement(&m, |ent| ent == "plugin:stairs").is_ok());
    assert_eq!(PluginManifest::parse(&m.to_json(), None).unwrap(), m);
    // An explicit entitlement wins; a free price needs nothing.
    let m = with_market(r#""entitlement": "plugin:acme.suite","#).unwrap();
    assert_eq!(
        m.required_entitlement().as_deref(),
        Some("plugin:acme.suite")
    );
    let m = with_market(r#""price": {"kind": "free"},"#).unwrap();
    assert!(!m.is_paid() && m.required_entitlement().is_none());
    let m =
        with_market(r#""price": {"kind": "subscription", "amount": 99000, "period": "month"},"#)
            .unwrap();
    assert!(m.is_paid());
    // Malformed fields are refused with the reason.
    for bad in [
        r#""price": {"kind": "gift"},"#,
        r#""price": {"kind": "one_time"},"#,
        r#""price": {"kind": "subscription", "amount": 1, "period": "day"},"#,
        r#""entitlement": "Plugin Stairs","#,
        r#""publisher": 3,"#,
    ] {
        assert!(with_market(bad).is_err(), "{bad}");
    }
}

#[test]
fn missing_fields_are_all_named() {
    let e = PluginManifest::parse("{}", None).unwrap_err();
    for f in ["`id`", "`name`", "`version`", "`author`", "`commands`"] {
        assert!(e.contains(&format!("missing {f}")), "{f}: {e}");
    }
    assert!(!e.contains("description"), "description is optional: {e}");
    let e = PluginManifest::parse(
        r#"{"id": "a", "name": "A", "version": "1", "author": "me", "commands": [{"id": "c"}]}"#,
        None,
    )
    .unwrap_err();
    assert!(e.contains("commands[0]: missing `label`"), "{e}");
    assert!(e.contains("commands[0]: missing `run`"), "{e}");
    let e = PluginManifest::parse(
        r#"{"id": "a", "name": " ", "version": "1", "author": "me", "commands": []}"#,
        None,
    )
    .unwrap_err();
    assert!(e.contains("missing `name`") && e.contains("empty"), "{e}");
    // Not JSON, not an object, wrong types: errors, never a panic.
    assert!(PluginManifest::parse("{", None)
        .unwrap_err()
        .contains("not JSON"));
    assert!(PluginManifest::parse("[1]", None).is_err());
    let e = PluginManifest::parse(
        r#"{"id": 3, "name": "A", "version": "1", "author": "me", "commands": [5]}"#,
        None,
    )
    .unwrap_err();
    assert!(
        e.contains("`id` must be text") && e.contains("must be an object"),
        "{e}"
    );
}

fn one_command(cmd: &str) -> String {
    format!(r#"{{"id": "p", "name": "P", "version": "1", "author": "me", "commands": [{cmd}]}}"#)
}

#[test]
fn bad_entry_points_keys_groups_and_ids() {
    let root = temp("entries");
    write(&root, "main.py", "def f(doc): pass\n");
    write(&root, "pkg/__init__.py", "def g(): pass\n");
    write(&root, "run.py", "result = 2\n");
    let ok = |run: &str| {
        PluginManifest::parse(
            &one_command(&format!(r#"{{"id": "c", "label": "C", "run": "{run}"}}"#)),
            Some(&root),
        )
    };
    assert!(ok("main:f").is_ok());
    assert!(ok("pkg:g").is_ok(), "a package works");
    assert!(ok("run.py").is_ok());
    for (run, says) in [
        ("main", "module:function"),
        ("main:", "not a Python function name"),
        ("main:1f", "not a Python function name"),
        ("ma-in:f", "not a Python module name"),
        ("other:f", "no other.py"),
        ("pkg.sub:f", "no pkg/sub.py"),
        ("missing.py", "no such file"),
        ("../evil.py", "inside the plugin folder"),
        ("/etc/evil.py", "inside the plugin folder"),
    ] {
        let e = ok(run).unwrap_err();
        assert!(e.contains(says), "{run}: {e}");
    }
    let bad = |cmd: &str| PluginManifest::parse(&one_command(cmd), Some(&root)).unwrap_err();
    assert!(
        bad(r#"{"id": "c", "label": "C", "run": "main:f", "key": "B"}"#).contains("two-letter")
    );
    assert!(
        bad(r#"{"id": "c", "label": "C", "run": "main:f", "key": "B1"}"#).contains("two-letter")
    );
    assert!(
        bad(r#"{"id": "c", "label": "C", "run": "main:f", "group": "toolbar"}"#).contains("group")
    );
    assert!(
        bad(r#"{"id": "c", "label": "C", "run": "main:f", "timeout_s": 900}"#)
            .contains("timeout_s")
    );
    assert!(bad(r#"{"id": "Big Id", "label": "C", "run": "main:f"}"#).contains("lower-case"));
    let e = PluginManifest::parse(
        r#"{"id": "p", "name": "P", "version": "1", "author": "me", "commands": [
            {"id": "c", "label": "C", "run": "main:f"}, {"id": "c", "label": "D", "run": "main:f"}]}"#,
        Some(&root),
    )
    .unwrap_err();
    assert!(e.contains("duplicate command id `c`"), "{e}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_loader_lists_good_broken_disabled_and_duplicate_plugins() {
    // A temporary FERRO_HOME, as the app uses it.
    let home = temp("home");
    std::env::set_var("FERRO_HOME", &home);
    let root = plugins_dir().unwrap();
    std::env::remove_var("FERRO_HOME");
    assert_eq!(root, home.join("plugins"));
    assert!(
        scan(&root).is_empty(),
        "no folder yet: no plugins, no error"
    );
    good_plugin(&root, "a-good");
    write(&root.join("b-broken"), MANIFEST_FILE, r#"{"id": "broken""#);
    write(&root.join("c-empty"), "notes.txt", "no manifest");
    good_plugin(&root, "d-copy"); // same id as a-good
    let off = good_plugin(&root, "e-off");
    write(&off, MANIFEST_FILE, &GOOD.replace("box-tools", "off-tools"));
    set_enabled(&off, false).unwrap();
    write(&root.join(".install-x"), MANIFEST_FILE, GOOD); // staging: skipped
    write(&root, "loose.txt", "a file, not a plugin");
    let list = scan(&root);
    let folders: Vec<&str> = list.iter().map(|p| p.folder.as_str()).collect();
    assert_eq!(
        folders,
        ["a-good", "b-broken", "c-empty", "d-copy", "e-off"]
    );
    assert!(list[0].active().is_some());
    assert!(list[1].manifest.as_ref().unwrap_err().contains("not JSON"));
    assert!(list[2]
        .manifest
        .as_ref()
        .unwrap_err()
        .contains("no plugin.json"));
    let dup = list[3].manifest.as_ref().unwrap_err();
    assert!(
        dup.contains("`a-good` already has the id `box-tools`"),
        "{dup}"
    );
    assert!(!list[4].enabled && list[4].active().is_none());
    assert!(
        list[4].manifest.is_ok(),
        "disabled, still read for the list"
    );
    assert!(find(&list, "box-tools", "box").is_some());
    assert!(find(&list, "off-tools", "box").is_none(), "disabled");
    assert!(find(&list, "box-tools", "nope").is_none());
    set_enabled(&off, true).unwrap();
    set_enabled(&off, true).unwrap(); // twice is fine
    assert!(scan(&root)[4].active().is_some());
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn built_in_keys_win_and_the_first_plugin_keeps_a_shared_key() {
    let root = temp("keys");
    for (folder, id, key) in [
        ("a", "alpha", "BX"),
        ("b", "beta", "BX"),
        ("c", "gamma", "WA"),
    ] {
        let d = root.join(folder);
        write(
            &d,
            MANIFEST_FILE,
            &format!(
                r#"{{"id": "{id}", "name": "{id}", "version": "1", "author": "me", "commands": [{{"id": "c", "label": "Go", "key": "{key}", "run": "main:f"}}, {{"id": "nokey", "label": "No key", "run": "main:f"}}]}}"#
            ),
        );
        write(&d, "main.py", "def f(): pass\n");
    }
    let list = scan(&root);
    let (keys, warnings) = assign_keys(&list, |k| (k == "WA").then(|| "Wall".to_string()));
    assert_eq!(
        keys,
        [KeyBinding {
            key: "BX".into(),
            plugin: "alpha".into(),
            command: "c".into()
        }]
    );
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings[0].contains("already used by plugin alpha"),
        "{warnings:?}"
    );
    assert!(warnings[1].contains("Ferro's Wall") && warnings[1].contains("built-in keeps it"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ids_from_names_and_the_template() {
    assert_eq!(plugin_id_for("Hộp ở gốc"), "hop-o-goc");
    assert_eq!(plugin_id_for("  Đường  ống!! "), "duong-ong");
    assert_eq!(plugin_id_for("***"), "my-plugin");
    assert_eq!(plugin_id_for("Box Tools 2"), "box-tools-2");
    let root = temp("template");
    let a = create_plugin(&root, "My box", "TaD").unwrap();
    let b = create_plugin(&root, "My box", "").unwrap();
    assert_eq!(a.file_name().unwrap(), "my-box");
    assert_eq!(b.file_name().unwrap(), "my-box-2");
    let list = scan(&root);
    assert_eq!(list.len(), 2);
    let m = list[0].active().unwrap();
    assert_eq!((m.name.as_str(), m.author.as_str()), ("My box", "TaD"));
    assert_eq!(list[1].active().unwrap().author, "me");
    assert_eq!(m.commands[0].label, "Box at origin");
    assert_eq!(m.commands[0].key.as_deref(), Some("BX"));
    let readme = std::fs::read_to_string(a.join("README.md")).unwrap();
    assert!(readme.starts_with("# My box") && readme.contains("Tiếng Việt"));
    assert!(readme.contains("English"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_watch_sees_edits_once_a_second() {
    let root = temp("watch");
    let d = good_plugin(&root, "p");
    let t0 = Instant::now();
    let mut w = PluginWatch::new(&root, t0);
    assert_eq!(w.root(), root.as_path());
    assert!(!w.poll(t0 + POLL_INTERVAL), "nothing changed");
    // An edit (size changes, so even a coarse mtime sees it).
    write(&d, "main.py", "def box(doc):\n    return 1\n");
    assert!(!w.poll(t0 + POLL_INTERVAL), "not before the next interval");
    assert!(w.poll(t0 + 2 * POLL_INTERVAL));
    assert!(!w.poll(t0 + 3 * POLL_INTERVAL), "reported once");
    // Caches do not count; a new plugin and disabling do.
    write(&d, "__pycache__/main.cpython-312.pyc", "x");
    assert!(!w.poll(t0 + 4 * POLL_INTERVAL));
    good_plugin(&root, "q");
    assert!(w.poll(t0 + 5 * POLL_INTERVAL));
    set_enabled(&d, false).unwrap();
    assert!(w.poll(t0 + 6 * POLL_INTERVAL));
    // A reload by hand takes the state as loaded.
    write(&d, "extra.py", "");
    w.loaded(t0 + 6 * POLL_INTERVAL);
    assert!(!w.poll(t0 + 8 * POLL_INTERVAL));
    // Same size, new time: still seen.
    std::thread::sleep(Duration::from_millis(20));
    write(&d, "extra.py", "");
    let f = std::fs::File::options()
        .write(true)
        .open(d.join("extra.py"))
        .unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    assert!(w.poll(t0 + 9 * POLL_INTERVAL));
    // A missing folder is an empty, unchanging state.
    let gone = root.join("none");
    let mut w = PluginWatch::new(&gone, t0);
    assert!(!w.poll(t0 + POLL_INTERVAL));
    let _ = std::fs::remove_dir_all(root);
}

/// A zip with stored (`false`) or deflated (`true`) entries.
fn zip_bytes(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data, deflate) in entries {
        let body = if *deflate {
            let mut e =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(data).unwrap();
            e.finish().unwrap()
        } else {
            data.to_vec()
        };
        let offset = out.len() as u32;
        let method: u16 = if *deflate { 8 } else { 0 };
        let mut local = Vec::new();
        local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        local.extend_from_slice(&[20, 0, 0, 0]);
        local.extend_from_slice(&method.to_le_bytes());
        local.extend_from_slice(&[0; 8]); // time, date, crc
        local.extend_from_slice(&(body.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&local);
        out.extend_from_slice(&body);
        let mut c = Vec::new();
        c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        c.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        c.extend_from_slice(&method.to_le_bytes());
        c.extend_from_slice(&[0; 8]);
        c.extend_from_slice(&(body.len() as u32).to_le_bytes());
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(&(name.len() as u16).to_le_bytes());
        c.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attrs
        c.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&c);
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

const LIMITS: ZipLimits = ZipLimits {
    max_files: 100,
    max_bytes: 1 << 20,
};

#[test]
fn unzip_refuses_paths_outside_and_huge_contents() {
    let dir = temp("unzip");
    let zip = dir.join("p.zip");
    std::fs::write(
        &zip,
        zip_bytes(&[
            ("p/", b"", false),
            ("p/a.txt", b"hello", false),
            ("p/b.txt", &[b'x'; 5000], true),
        ]),
    )
    .unwrap();
    let out = dir.join("out");
    std::fs::create_dir(&out).unwrap();
    assert_eq!(extract_zip(&zip, &out, &LIMITS).unwrap(), 2);
    assert_eq!(
        std::fs::read_to_string(out.join("p/a.txt")).unwrap(),
        "hello"
    );
    assert_eq!(std::fs::read(out.join("p/b.txt")).unwrap().len(), 5000);
    for evil in [
        "../evil.txt",
        "/tmp/evil.txt",
        "a/../../evil.txt",
        "C:/evil.txt",
        "..\\evil.txt",
    ] {
        std::fs::write(&zip, zip_bytes(&[(evil, b"x", false)])).unwrap();
        let e = extract_zip(&zip, &out, &LIMITS).unwrap_err();
        assert!(e.contains("outside"), "{evil}: {e}");
    }
    assert!(!dir.join("evil.txt").exists());
    // A zip bomb: small, but more than the limit unpacked.
    std::fs::write(&zip, zip_bytes(&[("big", &vec![0u8; 2 << 20], true)])).unwrap();
    assert!(std::fs::metadata(&zip).unwrap().len() < 100_000);
    assert!(extract_zip(&zip, &out, &LIMITS)
        .unwrap_err()
        .contains("more than 1 MB"));
    let few = ZipLimits {
        max_files: 1,
        ..LIMITS
    };
    std::fs::write(&zip, zip_bytes(&[("a", b"1", false), ("b", b"2", false)])).unwrap();
    assert!(extract_zip(&zip, &out, &few)
        .unwrap_err()
        .contains("more than 1 files"));
    // Not a zip, truncated, empty: errors, never a panic.
    std::fs::write(&zip, b"not a zip at all").unwrap();
    assert!(extract_zip(&zip, &out, &LIMITS)
        .unwrap_err()
        .contains("not a valid .zip"));
    let good = zip_bytes(&[("a.txt", b"hello", false)]);
    std::fs::write(&zip, &good[..good.len() - 30]).unwrap();
    assert!(extract_zip(&zip, &out, &LIMITS).is_err());
    std::fs::write(&zip, b"").unwrap();
    assert!(extract_zip(&zip, &out, &LIMITS).is_err());
    // A zip made by another tool (Python's zipfile, deflated, with a
    // directory entry).
    if interpreter_works(Path::new("python3")) {
        let script = "import sys, zipfile\n\
                      with zipfile.ZipFile(sys.argv[1], 'w', zipfile.ZIP_DEFLATED) as z:\n    \
                      z.writestr('q/', '')\n    \
                      z.writestr('q/plugin.json', '{}' * 100)\n    \
                      z.writestr('q/main.py', 'x = 1')\n";
        let made = std::process::Command::new("python3")
            .args(["-I", "-c", script])
            .arg(&zip)
            .status()
            .is_ok_and(|s| s.success());
        assert!(made);
        let out = dir.join("py");
        std::fs::create_dir(&out).unwrap();
        assert_eq!(extract_zip(&zip, &out, &LIMITS).unwrap(), 2);
        assert_eq!(
            std::fs::read_to_string(out.join("q/plugin.json"))
                .unwrap()
                .len(),
            200
        );
    }
    assert_eq!(unzip::safe_path("./a/./b"), Some(PathBuf::from("a/b")));
    assert_eq!(unzip::safe_path(""), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn install_from_zip_and_folder_then_replace_and_remove() {
    let dir = temp("install");
    let root = dir.join("plugins");
    let main = b"def box(doc):\n    pass\n";
    let zip = dir.join("box.zip");
    std::fs::write(
        &zip,
        zip_bytes(&[
            ("box-tools/plugin.json", GOOD.as_bytes(), true),
            ("box-tools/main.py", main, false),
            ("box-tools/scripts/tidy.py", b"result = 1\n", false),
        ]),
    )
    .unwrap();
    let staged = stage_install(&zip, &root).unwrap();
    assert_eq!(staged.manifest.author, "TaD");
    assert_eq!(staged.replaces, None);
    assert!(scan(&root).is_empty(), "staging is not a plugin");
    let at = finish_install(staged, &root).unwrap();
    assert_eq!(at, root.join("box-tools"));
    let list = scan(&root);
    assert_eq!(list.len(), 1);
    assert!(list[0].active().is_some());
    assert_eq!(
        std::fs::read_dir(&root).unwrap().count(),
        1,
        "no staging left"
    );
    // A newer version from a folder (files at its top) replaces it, even
    // installed under another folder name; a disabled plugin is enabled.
    set_enabled(&at, false).unwrap();
    let src = dir.join("src");
    write(&src, MANIFEST_FILE, &GOOD.replace("0.1.0", "0.2.0"));
    write(&src, "main.py", "def box(doc):\n    pass\n");
    write(&src, "scripts/tidy.py", "result = 2\n");
    let staged = stage_install(&src, &root).unwrap();
    assert_eq!(staged.replaces.as_deref(), Some("0.1.0"));
    finish_install(staged, &root).unwrap();
    let list = scan(&root);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].active().unwrap().version, "0.2.0");
    // A different plugin in a folder named like the id is kept.
    let other = root.join("x-tools");
    write(&other, MANIFEST_FILE, &GOOD.replace("box-tools", "other"));
    write(&other, "main.py", "def box(doc): pass\n");
    write(&other, "scripts/tidy.py", "");
    write(&src, MANIFEST_FILE, &GOOD.replace("box-tools", "x-tools"));
    let at = finish_install(stage_install(&src, &root).unwrap(), &root).unwrap();
    assert_eq!(at, root.join("x-tools-2"));
    assert_eq!(scan(&root).len(), 3);
    // Broken sources leave nothing behind.
    let broken = dir.join("broken.zip");
    std::fs::write(&broken, zip_bytes(&[("readme.txt", b"hi", false)])).unwrap();
    assert!(stage_install(&broken, &root)
        .unwrap_err()
        .contains("not a Ferro plugin"));
    std::fs::write(&broken, zip_bytes(&[("plugin.json", b"{}", false)])).unwrap();
    assert!(stage_install(&broken, &root)
        .unwrap_err()
        .contains("broken"));
    std::fs::write(
        &broken,
        zip_bytes(&[("../plugin.json", GOOD.as_bytes(), false)]),
    )
    .unwrap();
    assert!(stage_install(&broken, &root)
        .unwrap_err()
        .contains("outside"));
    assert!(stage_install(&dir.join("x.txt"), &root).is_err());
    let cancelled = stage_install(&zip, &root).unwrap();
    cancel_install(cancelled);
    let left = stage_install(&zip, &root).unwrap();
    clean_staging(&root);
    assert!(!left.staging.exists());
    assert_eq!(scan(&root).len(), 3);
    assert!(std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .all(|e| !e.file_name().to_string_lossy().starts_with('.')));
    // Removing: only folders directly inside the plugins folder.
    assert!(remove_plugin(&root, &dir).is_err());
    assert!(remove_plugin(&root, &root.join("x-tools/scripts")).is_err());
    remove_plugin(&root, &root.join("x-tools-2")).unwrap();
    assert_eq!(scan(&root).len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

/// A headless server with python3, or `None` (skipped) without one.
fn server() -> Option<Server> {
    let config = PythonConfig {
        interpreter: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        sdk: None,
    };
    if !interpreter_works(&config.interpreter) {
        eprintln!("skipped: no python3 (3.9+) on PATH");
        return None;
    }
    let mut s = Server::headless(Actor::user("user"));
    s.python = config;
    Some(s)
}

fn plugin_with(root: &Path, main: &str, commands: &str) -> Plugin {
    let d = root.join("t");
    write(
        &d,
        MANIFEST_FILE,
        &format!(
            r#"{{"id": "t", "name": "Test plugin", "version": "1", "author": "me", "commands": [{commands}]}}"#
        ),
    );
    write(&d, "main.py", main);
    scan(root).remove(0)
}

#[test]
fn plugin_command_from_the_template_makes_a_box_in_one_undo_step() {
    let Some(mut s) = server() else { return };
    let root = temp("run-template");
    create_plugin(&root, "Box", "me").unwrap();
    let plugin = scan(&root).remove(0);
    let r = run_command_blocking(
        &mut s.session,
        &s.python,
        &plugin,
        "box_at_origin",
        &RunControl::new(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(
        r["stdout"], "selected before: []\n",
        "headless: no selection"
    );
    assert_eq!(s.session.doc().len(), 1);
    // The box is 3 m high: `ferro.ask` returned its default headless.
    let id = r["changed"][0].as_u64().unwrap();
    let e = s.call("get_element", &json!({"element": id})).unwrap();
    assert!(
        e.to_string().contains("3000") || e.to_string().contains("3.0"),
        "{e}"
    );
    let j = s.session.journal();
    assert_eq!(j.len(), 1);
    assert_eq!(j[0].transaction, "Box at origin");
    assert_eq!(j[0].actor.kind, ActorKind::Plugin { id: "box".into() });
    assert_eq!(j[0].actor.name, "Box");
    s.call("undo", &json!({})).unwrap();
    assert_eq!(s.session.doc().len(), 0, "one undo step");
    let _ = std::fs::remove_dir_all(root);
}

const BOX: &str =
    "    doc.call('create_box', origin=[0, 0, 0], width='1m', depth='1m', height='1m')\n";

#[test]
fn plugin_command_exception_rolls_back_and_scripts_return_results() {
    let Some(mut s) = server() else { return };
    let root = temp("run-fail");
    let plugin = plugin_with(
        &root,
        &format!("import helper\n\ndef bad(doc):\n{BOX}{BOX}    raise RuntimeError('oops')\n\ndef plain():\n    return helper.VALUE\n"),
        r#"{"id": "bad", "label": "Bad", "run": "main:bad"},
           {"id": "plain", "label": "Plain", "run": "main:plain"},
           {"id": "script", "label": "Script", "run": "s.py"}"#,
    );
    write(&plugin.dir, "helper.py", "VALUE = 41\n");
    write(&plugin.dir, "s.py", &format!("import os\nif True:\n{BOX}result = [os.path.basename(os.getcwd()), len(doc.elements())]\n"));
    let plugin = scan(&root).remove(0);
    let r = run_command_blocking(
        &mut s.session,
        &s.python,
        &plugin,
        "bad",
        &RunControl::new(),
    )
    .unwrap_err();
    assert_eq!(r["failures"][0]["code"], python::FAILED);
    assert!(
        r["stderr"].as_str().unwrap().contains("RuntimeError: oops"),
        "{r}"
    );
    assert!(
        r["stderr"].as_str().unwrap().contains("main.py"),
        "the traceback names the plugin file"
    );
    assert_eq!(s.session.doc().len(), 0, "rolled back");
    assert!(s.session.journal().is_empty());
    // A function without `doc`, importing a module next to it.
    let r = run_command_blocking(
        &mut s.session,
        &s.python,
        &plugin,
        "plain",
        &RunControl::new(),
    )
    .unwrap();
    assert_eq!(r["result"], 41);
    // A script file: runs in the plugin folder, `doc` predefined.
    let r = run_command_blocking(
        &mut s.session,
        &s.python,
        &plugin,
        "script",
        &RunControl::new(),
    )
    .unwrap();
    assert_eq!(r["result"], json!(["t", 1]));
    assert!(run_command_blocking(
        &mut s.session,
        &s.python,
        &plugin,
        "nope",
        &RunControl::new()
    )
    .is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn plugin_command_cancel_stops_and_rolls_back() {
    let Some(mut s) = server() else { return };
    let root = temp("run-cancel");
    let plugin = plugin_with(
        &root,
        &format!("def spin(doc):\n{BOX}    while True:\n        pass\n"),
        r#"{"id": "spin", "label": "Spin", "run": "main:spin", "timeout_s": 60}"#,
    );
    let control = RunControl::new();
    let c = control.clone();
    let canceller = std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(20);
        while !c.is_running() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(300));
        // Another plugin may not stop it; the user may.
        let other = Actor {
            kind: ActorKind::Plugin { id: "other".into() },
            name: "Other".into(),
        };
        let refused = c.cancel(&other).is_err();
        (refused, c.cancel(&Actor::user("user")))
    });
    let started = Instant::now();
    let r = run_command_blocking(&mut s.session, &s.python, &plugin, "spin", &control).unwrap_err();
    let (refused, cancelled) = canceller.join().unwrap();
    assert!(refused, "only the user or the same plugin may cancel");
    assert_eq!(cancelled.unwrap()["cancelled"], true);
    assert!(started.elapsed() < Duration::from_secs(30));
    assert_eq!(r["failures"][0]["code"], python::CANCELLED, "{r}");
    assert_eq!(s.session.doc().len(), 0);
    assert!(!s.session.in_group());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn headless_ask_and_selection() {
    let Some(mut s) = server() else { return };
    let code = "result = [ferro.ask('Height', default='3m'), ferro.ask('Name'), ferro.ask('N', default=5), doc.selection(), len(doc.selection), doc.select([])]\n";
    let r = s
        .call(python::TOOL, &json!({ "code": code }))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        r["result"],
        json!(["3m", null, "5", [], 0, {"selected": 0}]),
        "{r}"
    );
    assert_eq!(
        python::headless_answer(python::GET_USER_INPUT, &json!({"default": "2m"})),
        Some(json!({"value": "2m"}))
    );
    assert_eq!(python::headless_answer("create_box", &json!({})), None);
    assert_eq!(
        python::answer_timeout(python::GET_USER_INPUT, &json!({})),
        python::USER_INPUT_TIMEOUT
    );
}
