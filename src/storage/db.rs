use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("failed to determine local data directory")]
    MissingDataDir,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: i64,
    pub question_number: Option<String>,
    pub question_type: String,
    pub question_text: String,
    pub answer: String,
    pub explanation: String,
    pub confidence: f32,
    pub verification_status: String,
    pub model_name: String,
    pub filled_status: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct NewHistoryEntry {
    pub question_number: Option<String>,
    pub question_type: String,
    pub question_text: String,
    pub answer: String,
    pub explanation: String,
    pub confidence: f32,
    pub verification_status: String,
    pub model_name: String,
    pub filled_status: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

pub struct HistoryDb {
    conn: Connection,
}

impl HistoryDb {
    pub fn default_path() -> Result<PathBuf, StorageError> {
        let local_app_data =
            std::env::var_os("LOCALAPPDATA").ok_or(StorageError::MissingDataDir)?;
        Ok(PathBuf::from(local_app_data)
            .join("KNUDiagnosis")
            .join("history.db"))
    }

    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<(), StorageError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS qa_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                question_number TEXT,
                question_type TEXT NOT NULL,
                question_text TEXT NOT NULL,
                answer TEXT NOT NULL,
                explanation TEXT NOT NULL,
                confidence REAL NOT NULL,
                verification_status TEXT NOT NULL,
                model_name TEXT NOT NULL,
                filled_status TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            "#,
        )?;
        Ok(())
    }

    pub fn insert(&self, entry: &NewHistoryEntry) -> Result<i64, StorageError> {
        self.conn.execute(
            r#"
            INSERT INTO qa_history (
                question_number, question_type, question_text, answer, explanation,
                confidence, verification_status, model_name, filled_status,
                input_tokens, output_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                entry.question_number,
                entry.question_type,
                entry.question_text,
                entry.answer,
                entry.explanation,
                entry.confidence,
                entry.verification_status,
                entry.model_name,
                entry.filled_status,
                entry.input_tokens,
                entry.output_tokens,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_recent(&self, query: &str, limit: usize) -> Result<Vec<HistoryEntry>, StorageError> {
        let trimmed = query.trim();
        let mut entries = Vec::new();
        if trimmed.is_empty() {
            let mut stmt = self.conn.prepare(
                r#"
                SELECT id, question_number, question_type, question_text, answer, explanation,
                       confidence, verification_status, model_name, filled_status,
                       input_tokens, output_tokens, created_at
                FROM qa_history
                ORDER BY id DESC
                LIMIT ?1
                "#,
            )?;
            let rows = stmt.query_map(params![limit as i64], map_row)?;
            for row in rows {
                entries.push(row?);
            }
        } else {
            let pattern = format!("%{trimmed}%");
            let mut stmt = self.conn.prepare(
                r#"
                SELECT id, question_number, question_type, question_text, answer, explanation,
                       confidence, verification_status, model_name, filled_status,
                       input_tokens, output_tokens, created_at
                FROM qa_history
                WHERE question_text LIKE ?1 OR answer LIKE ?1 OR explanation LIKE ?1
                ORDER BY id DESC
                LIMIT ?2
                "#,
            )?;
            let rows = stmt.query_map(params![pattern, limit as i64], map_row)?;
            for row in rows {
                entries.push(row?);
            }
        }
        Ok(entries)
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get(0)?,
        question_number: row.get(1)?,
        question_type: row.get(2)?,
        question_text: row.get(3)?,
        answer: row.get(4)?,
        explanation: row.get(5)?,
        confidence: row.get(6)?,
        verification_status: row.get(7)?,
        model_name: row.get(8)?,
        filled_status: row.get(9)?,
        input_tokens: row.get(10)?,
        output_tokens: row.get(11)?,
        created_at: row.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_and_searches_history_in_memory() {
        let db = HistoryDb::open_in_memory().expect("open in-memory db");
        let id = db
            .insert(&NewHistoryEntry {
                question_number: Some("1".to_string()),
                question_type: "predict_output".to_string(),
                question_text: "print(len([1, 2, 3]))".to_string(),
                answer: "3".to_string(),
                explanation: "List has 3 items".to_string(),
                confidence: 0.99,
                verification_status: "ai_reasoned".to_string(),
                model_name: "google/gemini-2.5-flash".to_string(),
                filled_status: "Filled 'Answer'".to_string(),
                input_tokens: 100,
                output_tokens: 40,
            })
            .expect("insert");
        assert!(id > 0);

        let all = db.list_recent("", 10).expect("list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].answer, "3");

        let filtered = db.list_recent("len([1", 10).expect("search");
        assert_eq!(filtered.len(), 1);

        let empty = db.list_recent("nonexistent_xyz", 10).expect("search empty");
        assert!(empty.is_empty());
    }
}
