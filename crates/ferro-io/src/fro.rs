use crate::json::{
    element_to_value, elements_and_ids, relationships_from_values, relationships_to_values,
    uid_map, FORMAT_VERSION,
};
use crate::IoError;
use ferro_core::model::{Document, ProjectInfo};
use ferro_core::ops::{DocumentChanged, JournalEntry};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS elements (uid TEXT PRIMARY KEY, ord INTEGER NOT NULL, json TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS elements_ord ON elements(ord);
CREATE TABLE IF NOT EXISTS journal (seq INTEGER PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS relationships (uid TEXT PRIMARY KEY, json TEXT NOT NULL);
";

/// An open `.fro` file for incremental saving (autosave, D-21 / Q-R16).
pub struct FroStore {
    conn: Connection,
    journal_written: u64,
}

impl FroStore {
    /// Create or open a `.fro` file.
    pub fn open(path: &Path) -> Result<FroStore, IoError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        let journal_written = conn
            .query_row("SELECT COALESCE(MAX(seq), 0) FROM journal", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0)
            .max(0) as u64;
        Ok(FroStore {
            conn,
            journal_written,
        })
    }

    /// Write everything (replacing the file's contents) in one transaction.
    pub fn write_all(&mut self, doc: &Document, journal: &[JournalEntry]) -> Result<(), IoError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM elements", [])?;
        write_meta(&tx, doc)?;
        let uids = uid_map(doc);
        {
            let mut ins =
                tx.prepare("INSERT INTO elements (uid, ord, json) VALUES (?1, ?2, ?3)")?;
            for e in doc.elements() {
                let v = element_to_value(e, &uids)?;
                ins.execute(params![e.uid.to_string(), e.id.0 as i64, v.to_string()])?;
            }
        }
        write_relationships(&tx, doc, &uids)?;
        tx.execute("DELETE FROM journal", [])?;
        {
            let mut ins = tx.prepare("INSERT INTO journal (seq, json) VALUES (?1, ?2)")?;
            for j in journal {
                ins.execute(params![j.seq as i64, serde_json::to_string(j)?])?;
            }
        }
        tx.commit()?;
        self.journal_written = journal.last().map(|j| j.seq).unwrap_or(0);
        Ok(())
    }

    /// Incremental save: only elements changed between `before` and `after`
    /// are written, plus new journal entries.
    pub fn sync(
        &mut self,
        before: &Document,
        after: &Document,
        journal: &[JournalEntry],
    ) -> Result<DocumentChanged, IoError> {
        let changed = DocumentChanged::between("save", before, after);
        let tx = self.conn.transaction()?;
        write_meta(&tx, after)?;
        let uids = uid_map(after);
        for id in changed.added.iter().chain(&changed.modified) {
            let e = after.get(*id).expect("listed by diff");
            let v = element_to_value(e, &uids)?;
            tx.execute(
                "INSERT INTO elements (uid, ord, json) VALUES (?1, ?2, ?3)
                 ON CONFLICT(uid) DO UPDATE SET ord = excluded.ord, json = excluded.json",
                params![e.uid.to_string(), e.id.0 as i64, v.to_string()],
            )?;
        }
        for id in &changed.deleted {
            if let Some(e) = before.get(*id) {
                tx.execute(
                    "DELETE FROM elements WHERE uid = ?1",
                    params![e.uid.to_string()],
                )?;
            }
        }
        if before.relationships().ne(after.relationships()) {
            write_relationships(&tx, after, &uids)?;
        }
        for j in journal.iter().filter(|j| j.seq > self.journal_written) {
            tx.execute(
                "INSERT OR REPLACE INTO journal (seq, json) VALUES (?1, ?2)",
                params![j.seq as i64, serde_json::to_string(j)?],
            )?;
        }
        tx.commit()?;
        if let Some(j) = journal.last() {
            self.journal_written = self.journal_written.max(j.seq);
        }
        Ok(changed)
    }

    pub fn load(&self) -> Result<(Document, Vec<JournalEntry>), IoError> {
        let get = |k: &str| -> Result<Option<String>, IoError> {
            Ok(self
                .conn
                .query_row("SELECT value FROM meta WHERE key = ?1", params![k], |r| {
                    r.get(0)
                })
                .optional()?)
        };
        if get("format")?.as_deref() != Some("ferro") {
            return Err(IoError::new("ferro.io.not_ferro", "not a Ferro file"));
        }
        let ver: u32 = get("format_version")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut project_v: Value =
            serde_json::from_str(&get("project")?.unwrap_or_else(|| "{}".into()))?;
        let mut stmt = self
            .conn
            .prepare("SELECT json FROM elements ORDER BY ord")?;
        let values = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .map(|s| -> Result<Value, IoError> { Ok(serde_json::from_str(&s?)?) })
            .collect::<Result<Vec<_>, _>>()?;
        let mut values = values;
        crate::migrate::migrate(ver, &mut project_v, &mut values)?;
        let project: ProjectInfo = serde_json::from_value(project_v)?;
        let (elements, id_of) = elements_and_ids(values)?;
        let mut stmt = self
            .conn
            .prepare("SELECT json FROM relationships ORDER BY uid")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .map(|s| -> Result<Value, IoError> { Ok(serde_json::from_str(&s?)?) })
            .collect::<Result<Vec<_>, _>>()?;
        let rows = relationships_from_values(&rows, &id_of)?;
        let doc = Document::from_parts(project, elements)
            .and_then(|d| d.with_relationships(rows))
            .map_err(|e| IoError::new("ferro.io.schema", e))?;
        let mut stmt = self.conn.prepare("SELECT json FROM journal ORDER BY seq")?;
        let journal = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(|s| s.ok().and_then(|s| serde_json::from_str(&s).ok()))
            .collect();
        Ok((doc, journal))
    }
}

/// Replace the relationship table (it is small; rows change rarely).
fn write_relationships(
    tx: &rusqlite::Transaction,
    doc: &Document,
    uids: &std::collections::HashMap<ferro_core::model::ElementId, ferro_core::model::Uid>,
) -> Result<(), IoError> {
    tx.execute("DELETE FROM relationships", [])?;
    let mut ins = tx.prepare("INSERT INTO relationships (uid, json) VALUES (?1, ?2)")?;
    for (r, v) in doc.relationships().zip(relationships_to_values(doc, uids)?) {
        ins.execute(params![r.uid.to_string(), v.to_string()])?;
    }
    Ok(())
}

fn write_meta(tx: &rusqlite::Transaction, doc: &Document) -> Result<(), IoError> {
    for (k, v) in [
        ("format", "ferro".to_string()),
        ("format_version", FORMAT_VERSION.to_string()),
        ("app_version", env!("CARGO_PKG_VERSION").to_string()),
        ("project", serde_json::to_string(doc.project())?),
    ] {
        tx.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![k, v],
        )?;
    }
    Ok(())
}

/// Save a complete `.fro` file. Writes to a temporary file first and renames,
/// so a crash never leaves a half-written file.
pub fn save_fro(path: &Path, doc: &Document, journal: &[JournalEntry]) -> Result<(), IoError> {
    let tmp = path.with_extension("fro.tmp");
    let _ = std::fs::remove_file(&tmp);
    {
        let mut store = FroStore::open(&tmp)?;
        store.write_all(doc, journal)?;
        store.conn.pragma_update(None, "journal_mode", "DELETE")?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Open a `.fro` file.
pub fn load_fro(path: &Path) -> Result<(Document, Vec<JournalEntry>), IoError> {
    if !path.exists() {
        return Err(IoError::new(
            "ferro.io.file",
            format!("{} does not exist", path.display()),
        ));
    }
    FroStore::open(path)?.load()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::tests::sample;
    use ferro_core::ops::{AnyCommand, ExecContext};
    use serde_json::json;

    #[test]
    fn full_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("house.fro");
        let s = sample();
        save_fro(&path, s.doc(), s.journal()).unwrap();
        let (doc, journal) = load_fro(&path).unwrap();
        assert_eq!(doc.len(), s.doc().len());
        assert_eq!(doc.project().name, "Nhà phố");
        assert_eq!(journal.len(), s.journal().len());
        for e in s.doc().elements() {
            assert_eq!(doc.find_uid(e.uid).unwrap().body, e.body);
        }
    }

    #[test]
    fn incremental_sync_writes_changes_and_deletions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inc.fro");
        let mut s = sample();
        let mut store = FroStore::open(&path).unwrap();
        store.write_all(s.doc(), s.journal()).unwrap();
        let before = s.doc().clone();
        s.execute(
            AnyCommand::from_tool(
                "move_elements",
                &json!({"elements":[3],"vector":{"x":1000,"y":0,"z":0}}),
            )
            .unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        s.execute(
            AnyCommand::from_tool("delete_elements", &json!({"elements":[1]})).unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        let changed = store.sync(&before, s.doc(), s.journal()).unwrap();
        assert_eq!(changed.deleted.len(), 1);
        drop(store);
        let (doc, journal) = load_fro(&path).unwrap();
        assert_eq!(doc.len(), 2);
        assert_eq!(journal.len(), s.journal().len());
        let moved = doc
            .find_uid(s.doc().get(ferro_core::ElementId(3)).unwrap().uid)
            .unwrap();
        assert!((moved.bounding_box().unwrap().min.x - 1000.0).abs() < 1e-6);
    }

    /// Two boxes (ids 4, 5) joined in the sample, ids shuffled on reload.
    fn with_rows() -> ferro_core::Session {
        let mut s = sample();
        let ctx = ExecContext::user();
        let run = |s: &mut ferro_core::Session, t: &str, a: serde_json::Value| {
            s.execute(AnyCommand::from_tool(t, &a).unwrap(), &ctx)
                .unwrap()
                .output
        };
        run(
            &mut s,
            "create_box",
            json!({"origin":{"x":9000,"y":0,"z":0},"width":1000,"depth":1000,"height":1000}),
        );
        run(
            &mut s,
            "create_box",
            json!({"origin":{"x":12000,"y":0,"z":0},"width":1000,"depth":1000,"height":1000}),
        );
        run(
            &mut s,
            "create_relationship",
            json!({"kind":"join","elements":[5, 4]}),
        );
        run(
            &mut s,
            "create_relationship",
            json!({"kind":"aggregate","elements":[3, 4, 5]}),
        );
        s
    }

    fn same_rows(a: &Document, b: &Document) {
        let key = |d: &Document| -> Vec<(String, Vec<ferro_core::model::Uid>)> {
            d.relationships()
                .map(|r| {
                    (
                        format!("{} {:?}", r.uid, r.kind),
                        r.elements.iter().map(|e| d.get(*e).unwrap().uid).collect(),
                    )
                })
                .collect()
        };
        assert_eq!(key(a), key(b));
    }

    #[test]
    fn relationships_survive_json_and_fro_by_uid() {
        let s = with_rows();
        let text = crate::json::document_to_json(s.doc()).unwrap();
        assert!(text.contains("\"relationships\""));
        same_rows(s.doc(), &crate::json::document_from_json(&text).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rel.fro");
        save_fro(&path, s.doc(), s.journal()).unwrap();
        same_rows(s.doc(), &load_fro(&path).unwrap().0);
        // A document without rows writes no `relationships` key (files
        // stay as they were).
        let plain = crate::json::document_to_json(sample().doc()).unwrap();
        assert!(!plain.contains("\"relationships\""));
    }

    #[test]
    fn incremental_sync_keeps_the_relationship_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rel-inc.fro");
        let mut s = with_rows();
        let mut store = FroStore::open(&path).unwrap();
        store.write_all(s.doc(), s.journal()).unwrap();
        let before = s.doc().clone();
        // Deleting box 4 drops both rows that link it.
        s.execute(
            AnyCommand::from_tool("delete_elements", &json!({"elements":[4]})).unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        assert_eq!(s.doc().relationships().count(), 0);
        store.sync(&before, s.doc(), s.journal()).unwrap();
        drop(store);
        let doc = load_fro(&path).unwrap().0;
        assert_eq!(doc.relationships().count(), 0);
        assert_eq!(doc.len(), s.doc().len());
    }

    #[test]
    fn a_row_pointing_at_a_missing_element_is_refused_on_load() {
        let s = with_rows();
        let text = crate::json::document_to_json(s.doc()).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        v["relationships"][0]["elements"][0] = json!("01a00000-0000-7000-8000-000000000000");
        let e = crate::json::document_from_json(&v.to_string()).unwrap_err();
        assert_eq!(e.code, "ferro.io.dangling_reference");
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        v["relationships"][0]["kind"] = json!("glue");
        let e = crate::json::document_from_json(&v.to_string()).unwrap_err();
        assert_eq!(e.code, "ferro.io.schema");
    }

    #[test]
    fn non_ferro_sqlite_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE x(a);")
            .unwrap();
        assert_eq!(load_fro(&path).unwrap_err().code, "ferro.io.not_ferro");
        assert_eq!(
            load_fro(&dir.path().join("missing.fro")).unwrap_err().code,
            "ferro.io.file"
        );
    }
}
