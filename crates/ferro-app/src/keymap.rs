//! Keyboard shortcuts as data (ADR 0004 §7, M5-11).
//!
//! Three schemes: **SketchUp** (one key per modelling tool), **Revit**
//! (two-letter codes typed in sequence, Autodesk's defaults) and **Ferro**
//! (both). In Ferro a single key acts at once; if a second letter follows
//! within [`SEQUENCE_WINDOW_S`] and the two letters form a Revit code, the
//! code wins. Switching tools has no side effect, so typing `S` `B`
//! briefly picks Scale and then Floor, as a Revit user expects.
//!
//! Users can override any binding in `~/.ferro/keymap.json`
//! (`{"scheme": "ferro", "bindings": {"WA": "wall", "K": "tape"}}`).

use crate::tools::ToolKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Seconds within which a second letter makes a two-letter code.
pub const SEQUENCE_WINDOW_S: f64 = 0.8;

/// What a shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    Polygon,
    PushPull,
    Offset,
    Move,
    Rotate,
    Scale,
    Eraser,
    Paint,
    Tape,
    Section,
    Wall,
    Floor,
    Door,
    Window,
    /// Hide the selection (view state).
    Hide,
    /// Show everything hidden.
    UnhideAll,
    ZoomExtents,
    Delete,
    /// Pin the selection (ADR 0008; Revit `PN`).
    Pin,
    /// Unpin the selection (Revit `UP`).
    Unpin,
    /// Solid tools (ADR 0007, M5-5.3).
    Subtract,
    Unite,
    Intersect,
    CutGeometry,
}

impl Target {
    pub const ALL: [Target; 29] = [
        Target::Select,
        Target::Line,
        Target::Rectangle,
        Target::Circle,
        Target::Arc,
        Target::Polygon,
        Target::PushPull,
        Target::Offset,
        Target::Move,
        Target::Rotate,
        Target::Scale,
        Target::Eraser,
        Target::Paint,
        Target::Tape,
        Target::Section,
        Target::Wall,
        Target::Floor,
        Target::Door,
        Target::Window,
        Target::Hide,
        Target::UnhideAll,
        Target::ZoomExtents,
        Target::Delete,
        Target::Pin,
        Target::Unpin,
        Target::Subtract,
        Target::Unite,
        Target::Intersect,
        Target::CutGeometry,
    ];

    /// The tool this target activates, if it is a tool.
    pub fn tool(self) -> Option<ToolKind> {
        Some(match self {
            Target::Select => ToolKind::Select,
            Target::Line => ToolKind::Line,
            Target::Rectangle => ToolKind::Rectangle,
            Target::Circle => ToolKind::Circle,
            Target::Arc => ToolKind::Arc,
            Target::Polygon => ToolKind::Polygon,
            Target::PushPull => ToolKind::PushPull,
            Target::Offset => ToolKind::Offset,
            Target::Move => ToolKind::Move,
            Target::Rotate => ToolKind::Rotate,
            Target::Scale => ToolKind::Scale,
            Target::Eraser => ToolKind::Eraser,
            Target::Paint => ToolKind::Paint,
            Target::Tape => ToolKind::Tape,
            Target::Section => ToolKind::Section,
            Target::Wall => ToolKind::Wall,
            Target::Floor => ToolKind::Floor,
            Target::Door => ToolKind::Door,
            Target::Window => ToolKind::Window,
            Target::Subtract => ToolKind::Subtract,
            Target::Unite => ToolKind::Unite,
            Target::Intersect => ToolKind::Intersect,
            Target::CutGeometry => ToolKind::CutGeometry,
            Target::Hide
            | Target::UnhideAll
            | Target::ZoomExtents
            | Target::Delete
            | Target::Pin
            | Target::Unpin => return None,
        })
    }

    /// The target that activates a tool.
    pub fn from_tool(k: ToolKind) -> Target {
        Target::ALL
            .into_iter()
            .find(|t| t.tool() == Some(k))
            .unwrap_or(Target::Select)
    }

    /// Label for menus and the command palette.
    pub fn label(self) -> &'static str {
        match self.tool() {
            Some(t) => t.label(),
            None => match self {
                Target::Hide => "Hide",
                Target::UnhideAll => "Unhide all",
                Target::ZoomExtents => "Zoom extents",
                Target::Pin => "Pin",
                Target::Unpin => "Unpin",
                _ => "Delete",
            },
        }
    }
}

/// Built-in shortcut sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyScheme {
    #[serde(rename = "sketchup")]
    SketchUp,
    Revit,
    #[default]
    Ferro,
}

impl KeyScheme {
    pub const ALL: [KeyScheme; 3] = [KeyScheme::Ferro, KeyScheme::SketchUp, KeyScheme::Revit];
    pub fn label(self) -> &'static str {
        match self {
            KeyScheme::SketchUp => "SketchUp",
            KeyScheme::Revit => "Revit",
            KeyScheme::Ferro => "Ferro (SketchUp + Revit codes)",
        }
    }
}

/// SketchUp defaults, plus Ferro's own keys for tools SketchUp has no key for.
const SKETCHUP: &[(&str, Target)] = &[
    ("SPACE", Target::Select),
    ("L", Target::Line),
    ("R", Target::Rectangle),
    ("C", Target::Circle),
    ("A", Target::Arc),
    ("P", Target::PushPull),
    ("F", Target::Offset),
    ("M", Target::Move),
    ("Q", Target::Rotate),
    ("S", Target::Scale),
    ("E", Target::Eraser),
    ("B", Target::Paint),
    ("T", Target::Tape),
    ("Z", Target::ZoomExtents),
    // Ferro additions (no SketchUp default).
    ("G", Target::Polygon),
    ("J", Target::Section),
    ("H", Target::Hide),
    ("SHIFT+H", Target::UnhideAll),
    ("DELETE", Target::Delete),
];

/// Autodesk's default Revit codes for the targets Ferro has.
const REVIT: &[(&str, Target)] = &[
    ("WA", Target::Wall),
    ("DR", Target::Door),
    ("WN", Target::Window),
    ("SB", Target::Floor),
    ("LI", Target::Line),
    ("MV", Target::Move),
    ("RO", Target::Rotate),
    ("OF", Target::Offset),
    ("RE", Target::Scale),
    ("PT", Target::Paint),
    ("DE", Target::Delete),
    ("HH", Target::Hide),
    ("HR", Target::UnhideAll),
    ("ZE", Target::ZoomExtents),
    ("ZF", Target::ZoomExtents),
    ("ZX", Target::ZoomExtents),
    ("PN", Target::Pin),
    ("UP", Target::Unpin),
    // Revit's Cut Geometry code; Revit has no codes for the booleans, so
    // Ferro adds free two-letter codes: SU(btract), UN(ite), IN(tersect).
    ("CG", Target::CutGeometry),
    ("SU", Target::Subtract),
    ("UN", Target::Unite),
    ("IN", Target::Intersect),
    ("DELETE", Target::Delete),
];

/// User file `~/.ferro/keymap.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KeymapFile {
    #[serde(default)]
    pub scheme: KeyScheme,
    /// Key (`"K"`, `"SHIFT+H"`) or two-letter code (`"WA"`) → target.
    #[serde(default)]
    pub bindings: BTreeMap<String, Target>,
}

/// The active bindings: keys (one letter or a named key) and two-letter codes.
#[derive(Debug, Clone, PartialEq)]
pub struct Keymap {
    pub scheme: KeyScheme,
    bindings: BTreeMap<String, Target>,
}

impl Keymap {
    /// The bindings of a preset.
    pub fn scheme(scheme: KeyScheme) -> Keymap {
        let mut bindings = BTreeMap::new();
        let sets: &[&[(&str, Target)]] = match scheme {
            KeyScheme::SketchUp => &[SKETCHUP],
            KeyScheme::Revit => &[REVIT],
            KeyScheme::Ferro => &[SKETCHUP, REVIT],
        };
        for set in sets {
            for (k, t) in set.iter() {
                bindings.insert((*k).to_string(), *t);
            }
        }
        Keymap { scheme, bindings }
    }

    /// A preset with the user's overrides applied. Invalid keys are
    /// returned (and skipped) so the app can say which ones.
    pub fn from_file(f: &KeymapFile) -> (Keymap, Vec<String>) {
        let mut k = Keymap::scheme(f.scheme);
        let mut bad = Vec::new();
        for (key, t) in &f.bindings {
            let key = key.trim().to_ascii_uppercase();
            if valid_key(&key) {
                k.bindings.insert(key, *t);
            } else {
                bad.push(key);
            }
        }
        (k, bad)
    }

    /// Target bound to a key or code.
    pub fn get(&self, key: &str) -> Option<Target> {
        self.bindings.get(key).copied()
    }

    /// Keys and codes for a target, single keys first (for labels).
    pub fn keys_for(&self, t: Target) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .bindings
            .iter()
            .filter(|(_, x)| **x == t)
            .map(|(k, _)| k.as_str())
            .collect();
        v.sort_by_key(|k| (k.len(), *k));
        v
    }

    /// Shortest label for a target's shortcut, e.g. "L" or "WA".
    pub fn label_for(&self, t: Target) -> String {
        self.keys_for(t)
            .first()
            .map(|k| match *k {
                "SPACE" => "Space".to_string(),
                k => k.to_string(),
            })
            .unwrap_or_default()
    }
}

/// A key name: one letter A–Z, a two-letter code, or a named key.
fn valid_key(k: &str) -> bool {
    let letters = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_uppercase());
    match k {
        "SPACE" | "DELETE" => true,
        _ if k.len() <= 2 && letters(k) => true,
        _ => k
            .strip_prefix("SHIFT+")
            .is_some_and(|r| r.len() == 1 && letters(r)),
    }
}

/// Turns key presses into targets, handling two-letter codes.
#[derive(Debug, Default)]
pub struct KeyResolver {
    last: Option<(char, f64)>,
}

impl KeyResolver {
    /// A letter (A–Z, any case) pressed at time `t` (seconds). Returns the
    /// target to run now, if any: a two-letter code when the previous letter
    /// was pressed within [`SEQUENCE_WINDOW_S`], else the letter's own binding.
    pub fn letter(&mut self, keymap: &Keymap, c: char, t: f64) -> Option<Target> {
        let c = c.to_ascii_uppercase();
        if !c.is_ascii_uppercase() {
            self.last = None;
            return None;
        }
        if let Some((p, t0)) = self.last {
            let dt = t - t0;
            if (0.0..=SEQUENCE_WINDOW_S).contains(&dt) {
                if let Some(target) = keymap.get(&format!("{p}{c}")) {
                    self.last = None;
                    return Some(target);
                }
            }
        }
        self.last = Some((c, t));
        keymap.get(&c.to_string())
    }

    /// A non-letter key (Space, Delete, Shift+letter) breaks a sequence.
    pub fn named(&mut self, keymap: &Keymap, name: &str) -> Option<Target> {
        self.last = None;
        keymap.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(k: &Keymap, keys: &[(char, f64)]) -> Vec<Option<Target>> {
        let mut r = KeyResolver::default();
        keys.iter().map(|(c, t)| r.letter(k, *c, *t)).collect()
    }

    #[test]
    fn sketchup_single_keys_act_at_once() {
        let k = Keymap::scheme(KeyScheme::SketchUp);
        assert_eq!(feed(&k, &[('l', 0.0)]), [Some(Target::Line)]);
        // No codes in the SketchUp preset: "W" "A" is nothing, then Arc.
        assert_eq!(
            feed(&k, &[('w', 0.0), ('a', 0.1)]),
            [None, Some(Target::Arc)]
        );
    }

    #[test]
    fn revit_codes_need_both_letters_in_time() {
        let k = Keymap::scheme(KeyScheme::Revit);
        assert_eq!(
            feed(&k, &[('w', 0.0), ('a', 0.3)]),
            [None, Some(Target::Wall)]
        );
        // Too slow: the second letter starts a new sequence.
        assert_eq!(feed(&k, &[('w', 0.0), ('a', 0.9)]), [None, None]);
        // Exactly at the window edge still counts.
        assert_eq!(
            feed(&k, &[('d', 0.0), ('r', SEQUENCE_WINDOW_S)]),
            [None, Some(Target::Door)]
        );
    }

    #[test]
    fn ferro_lets_a_revit_code_override_the_first_letter() {
        let k = Keymap::scheme(KeyScheme::Ferro);
        // S = Scale at once; S B = Floor.
        assert_eq!(
            feed(&k, &[('s', 0.0), ('b', 0.2)]),
            [Some(Target::Scale), Some(Target::Floor)]
        );
        // L then L (not a code): Line twice.
        assert_eq!(
            feed(&k, &[('l', 0.0), ('l', 0.2)]),
            [Some(Target::Line), Some(Target::Line)]
        );
        // After a code, the next letter starts fresh: W A then A = Arc.
        assert_eq!(
            feed(&k, &[('w', 0.0), ('a', 0.1), ('a', 0.2)]),
            [None, Some(Target::Wall), Some(Target::Arc)]
        );
        // Non-letters reset the sequence.
        let mut r = KeyResolver::default();
        assert_eq!(r.letter(&k, 'w', 0.0), None);
        assert_eq!(r.named(&k, "SPACE"), Some(Target::Select));
        assert_eq!(r.letter(&k, 'a', 0.1), Some(Target::Arc));
        assert_eq!(r.letter(&k, '1', 0.2), None);
    }

    #[test]
    fn every_target_has_a_shortcut_in_ferro_and_presets_do_not_clash() {
        let k = Keymap::scheme(KeyScheme::Ferro);
        for tool in ToolKind::ALL {
            assert_eq!(Target::from_tool(tool).tool(), Some(tool));
        }
        for t in Target::ALL {
            assert!(!k.keys_for(t).is_empty(), "{t:?} has no key");
        }
        // Each key / code is bound once per preset table.
        for set in [SKETCHUP, REVIT] {
            let mut keys: Vec<&str> = set.iter().map(|(k, _)| *k).collect();
            keys.sort();
            let n = keys.len();
            keys.dedup();
            assert_eq!(keys.len(), n, "duplicate key in a preset");
            assert!(set.iter().all(|(k, _)| valid_key(k)));
        }
        assert_eq!(k.label_for(Target::Select), "Space");
        assert_eq!(k.label_for(Target::Wall), "WA");
        assert_eq!(k.label_for(Target::Line), "L");
        assert_eq!(k.label_for(Target::Pin), "PN");
        assert_eq!(k.label_for(Target::Unpin), "UP");
        assert_eq!(k.label_for(Target::CutGeometry), "CG");
        assert_eq!(k.label_for(Target::Subtract), "SU");
        assert_eq!(k.label_for(Target::Unite), "UN");
        assert_eq!(k.label_for(Target::Intersect), "IN");
    }

    #[test]
    fn pin_codes_win_over_their_first_letter() {
        let k = Keymap::scheme(KeyScheme::Ferro);
        // P = Push/Pull at once, P N = Pin; U is unbound, U P = Unpin.
        assert_eq!(
            feed(&k, &[('p', 0.0), ('n', 0.3)]),
            [Some(Target::PushPull), Some(Target::Pin)]
        );
        assert_eq!(
            feed(&k, &[('u', 0.0), ('p', 0.3)]),
            [None, Some(Target::Unpin)]
        );
        assert_eq!(Target::Pin.label(), "Pin");
        assert_eq!(Target::Unpin.tool(), None);
    }

    #[test]
    fn user_file_overrides_and_reports_bad_keys() {
        let f: KeymapFile = serde_json::from_str(
            r#"{"scheme": "sketchup", "bindings": {"k": "tape", "WA": "wall", "CTRL+X": "move", "ABC": "line"}}"#,
        )
        .unwrap();
        let (k, bad) = Keymap::from_file(&f);
        assert_eq!(k.get("K"), Some(Target::Tape));
        assert_eq!(k.get("WA"), Some(Target::Wall));
        assert_eq!(k.get("L"), Some(Target::Line));
        assert_eq!(bad, ["ABC", "CTRL+X"]);
        // Empty file: the Ferro preset.
        let (k, bad) = Keymap::from_file(&KeymapFile::default());
        assert_eq!((k.scheme, bad.len()), (KeyScheme::Ferro, 0));
    }
}
