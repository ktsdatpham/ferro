use crate::json::{element_to_value, elements_from_values, uid_map, FORMAT_VERSION};
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
        let doc = Document::from_parts(project, elements_from_values(values)?)
            .map_err(|e| IoError::new("ferro.io.schema", e))?;
        let mut stmt = self.conn.prepare("SELECT json FROM journal ORDER BY seq")?;
        let journal = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(|s| s.ok().and_then(|s| serde_json::from_str(&s).ok()))
            .collect();
        Ok((doc, journal))
    }
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
