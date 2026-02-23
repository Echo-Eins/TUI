use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

// ── History Entry ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: i64,
    pub command: String,
    pub cwd: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub timestamp: i64,
    pub session_id: String,
    pub hostname: String,
}

// ── Command History (SQLite) ───────────────────────────────────────────────

pub struct CommandHistory {
    conn: Connection,
    session_id: String,
}

impl CommandHistory {
    /// Open or create the history database at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path.as_ref()).context("Failed to open history database")?;

        // Enable WAL mode for concurrent reads
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;

        // Create schema
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS command_history (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                command     TEXT NOT NULL,
                cwd         TEXT NOT NULL,
                exit_code   INTEGER,
                duration_ms INTEGER,
                timestamp   INTEGER NOT NULL,
                session_id  TEXT NOT NULL,
                hostname    TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cmd_text ON command_history(command);
            CREATE INDEX IF NOT EXISTS idx_cmd_ts ON command_history(timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_cmd_cwd ON command_history(cwd);",
        )?;

        // Generate a unique session ID
        let session_id = format!(
            "sess_{:x}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        Ok(Self { conn, session_id })
    }

    /// Open an in-memory database (for testing / when no persistent path is available).
    pub fn open_in_memory() -> Result<Self> {
        Self::open(":memory:")
    }

    /// Record a completed command into history.
    pub fn record(
        &self,
        command: &str,
        cwd: &str,
        exit_code: Option<i32>,
        duration_ms: Option<i64>,
        hostname: &str,
    ) -> Result<()> {
        let command = command.trim();
        if command.is_empty() {
            return Ok(());
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        self.conn.execute(
            "INSERT INTO command_history (command, cwd, exit_code, duration_ms, timestamp, session_id, hostname)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![command, cwd, exit_code, duration_ms, timestamp, &self.session_id, hostname],
        )?;

        Ok(())
    }

    /// Search by prefix for ghost text. Returns commands ranked by:
    /// recency * 0.4 + frequency * 0.3 + prefix_match * 0.3
    ///
    /// If `cwd_filter` is Some, boosts commands from that directory.
    pub fn search_prefix(
        &self,
        prefix: &str,
        cwd_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<HistoryEntry>> {
        let prefix = prefix.trim();
        if prefix.is_empty() {
            return Ok(Vec::new());
        }

        let like_pattern = format!("{}%", prefix.replace('%', "\\%").replace('_', "\\_"));

        // Get the max timestamp for normalization
        let max_ts: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(timestamp), 1) FROM command_history",
                [],
                |r| r.get(0),
            )
            .unwrap_or(1);

        let min_ts: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MIN(timestamp), 0) FROM command_history",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        let ts_range = (max_ts - min_ts).max(1) as f64;

        // Query all prefix matches, then rank in Rust for flexibility
        let mut stmt = self.conn.prepare(
            "SELECT id, command, cwd, exit_code, duration_ms, timestamp, session_id, hostname,
                    COUNT(*) OVER (PARTITION BY command) as freq
             FROM command_history
             WHERE command LIKE ?1 ESCAPE '\\'
             GROUP BY command
             ORDER BY timestamp DESC
             LIMIT ?2",
        )?;

        let entries: Vec<(HistoryEntry, i64)> = stmt
            .query_map(params![like_pattern, (limit * 3) as i64], |row| {
                Ok((
                    HistoryEntry {
                        id: row.get(0)?,
                        command: row.get(1)?,
                        cwd: row.get(2)?,
                        exit_code: row.get(3)?,
                        duration_ms: row.get(4)?,
                        timestamp: row.get(5)?,
                        session_id: row.get(6)?,
                        hostname: row.get(7)?,
                    },
                    row.get::<_, i64>(8)?, // freq
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let max_freq = entries.iter().map(|(_, f)| *f).max().unwrap_or(1).max(1) as f64;

        let mut scored: Vec<(HistoryEntry, f64)> = entries
            .into_iter()
            .map(|(entry, freq)| {
                let recency = (entry.timestamp - min_ts) as f64 / ts_range;
                let frequency = freq as f64 / max_freq;

                // Prefix match quality: exact prefix gets full score
                let prefix_len = prefix.len() as f64;
                let cmd_len = entry.command.len().max(1) as f64;
                let prefix_quality = (prefix_len / cmd_len).min(1.0);

                // CWD boost
                let cwd_boost = if cwd_filter.map_or(false, |cwd| cwd == entry.cwd) {
                    0.15
                } else {
                    0.0
                };

                // Successful command boost
                let success_boost = if entry.exit_code == Some(0) {
                    0.05
                } else {
                    0.0
                };

                let score = recency * 0.4
                    + frequency * 0.3
                    + prefix_quality * 0.3
                    + cwd_boost
                    + success_boost;

                (entry, score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        Ok(scored.into_iter().map(|(e, _)| e).collect())
    }

    /// Fuzzy substring search for Ctrl+R history search panel.
    pub fn search_fuzzy(
        &self,
        query: &str,
        cwd_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<HistoryEntry>> {
        let query = query.trim();
        if query.is_empty() {
            return self.get_recent(limit);
        }

        let like_pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));

        let cwd_clause = if cwd_filter.is_some() {
            "AND cwd = ?3"
        } else {
            ""
        };

        let sql = format!(
            "SELECT DISTINCT command, MAX(id) as id, cwd, exit_code, duration_ms, 
                    MAX(timestamp) as timestamp, session_id, hostname
             FROM command_history
             WHERE command LIKE ?1 ESCAPE '\\'
             {}
             GROUP BY command
             ORDER BY timestamp DESC
             LIMIT ?2",
            cwd_clause
        );

        let mut stmt = self.conn.prepare(&sql)?;

        let entries = if let Some(cwd) = cwd_filter {
            stmt.query_map(params![like_pattern, limit as i64, cwd], |row| {
                Ok(HistoryEntry {
                    id: row.get(1)?,
                    command: row.get(0)?,
                    cwd: row.get(2)?,
                    exit_code: row.get(3)?,
                    duration_ms: row.get(4)?,
                    timestamp: row.get(5)?,
                    session_id: row.get(6)?,
                    hostname: row.get(7)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect()
        } else {
            stmt.query_map(params![like_pattern, limit as i64], |row| {
                Ok(HistoryEntry {
                    id: row.get(1)?,
                    command: row.get(0)?,
                    cwd: row.get(2)?,
                    exit_code: row.get(3)?,
                    duration_ms: row.get(4)?,
                    timestamp: row.get(5)?,
                    session_id: row.get(6)?,
                    hostname: row.get(7)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect()
        };

        Ok(entries)
    }

    /// Get the most recent unique commands (for Up/Down cycling).
    pub fn get_recent(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT command, MAX(id) as id, cwd, exit_code, duration_ms,
                    MAX(timestamp) as timestamp, session_id, hostname
             FROM command_history
             GROUP BY command
             ORDER BY id DESC
             LIMIT ?1",
        )?;

        let entries = stmt
            .query_map(params![limit as i64], |row| {
                Ok(HistoryEntry {
                    id: row.get(1)?,
                    command: row.get(0)?,
                    cwd: row.get(2)?,
                    exit_code: row.get(3)?,
                    duration_ms: row.get(4)?,
                    timestamp: row.get(5)?,
                    session_id: row.get(6)?,
                    hostname: row.get(7)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    /// Get the total number of unique commands in history.
    pub fn count_unique(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT command) FROM command_history",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Get session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_search() {
        let history = CommandHistory::open_in_memory().unwrap();

        history
            .record(
                "systemctl restart nginx",
                "/home/user",
                Some(0),
                Some(100),
                "testhost",
            )
            .unwrap();
        history
            .record(
                "systemctl status nginx",
                "/home/user",
                Some(0),
                Some(50),
                "testhost",
            )
            .unwrap();
        history
            .record("ls -la", "/home/user", Some(0), Some(10), "testhost")
            .unwrap();

        let results = history.search_prefix("sys", None, 10).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].command.starts_with("sys"));

        let results = history.search_fuzzy("nginx", None, 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_get_recent() {
        let history = CommandHistory::open_in_memory().unwrap();

        history
            .record("cmd1", "/", Some(0), Some(10), "host")
            .unwrap();
        history
            .record("cmd2", "/", Some(0), Some(10), "host")
            .unwrap();
        history
            .record("cmd3", "/", Some(0), Some(10), "host")
            .unwrap();

        let recent = history.get_recent(2).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].command, "cmd3");
        assert_eq!(recent[1].command, "cmd2");
    }

    #[test]
    fn test_empty_command_not_recorded() {
        let history = CommandHistory::open_in_memory().unwrap();
        history.record("", "/", Some(0), Some(10), "host").unwrap();
        history
            .record("   ", "/", Some(0), Some(10), "host")
            .unwrap();
        assert_eq!(history.count_unique().unwrap(), 0);
    }
}
