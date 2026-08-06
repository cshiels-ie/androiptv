//! SQLite persistence. One connection behind a mutex (rusqlite's
//! Connection is Send but not Sync); heavy imports run inside
//! `spawn_blocking` so the UI never stalls.
//!
//! Schema is forward-compatible with EPG/VOD/series (kind column).

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

use crate::m3u::ParsedChannel;

#[derive(Debug, Clone, Serialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub source_type: String,
    pub source_url: String,
    pub xtream_base: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub id: i64,
    pub playlist_id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Channel {
    pub id: i64,
    pub playlist_id: i64,
    pub group_id: Option<i64>,
    pub name: String,
    pub url: String,
    pub logo_url: Option<String>,
    pub tvg_id: Option<String>,
    pub tvg_chno: Option<i64>,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportStats {
    pub channels: usize,
    pub groups: usize,
}

pub struct Db {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS playlists (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  source_type TEXT NOT NULL,
  source_url TEXT NOT NULL,
  xtream_base TEXT,
  xtream_user TEXT,
  xtream_pass TEXT,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS groups (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  UNIQUE(playlist_id, name)
);
CREATE TABLE IF NOT EXISTS channels (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,
  name TEXT NOT NULL,
  url TEXT NOT NULL,
  logo_url TEXT,
  tvg_id TEXT,
  tvg_chno INTEGER,
  kind TEXT NOT NULL DEFAULT 'live'
);
CREATE INDEX IF NOT EXISTS idx_ch_playlist ON channels(playlist_id);
CREATE INDEX IF NOT EXISTS idx_ch_group ON channels(group_id);
CREATE INDEX IF NOT EXISTS idx_ch_name ON channels(name COLLATE NOCASE);
"#;

const BATCH: usize = 5000;

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)
    }

    // ---------- playlists ----------

    pub fn create_playlist(
        &self,
        name: &str,
        source_type: &str,
        source_url: &str,
        xtream: Option<(&str, &str, &str)>,
    ) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO playlists (name, source_type, source_url, xtream_base, xtream_user, xtream_pass, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%s','now'))",
            params![
                name,
                source_type,
                source_url,
                xtream.map(|x| x.0),
                xtream.map(|x| x.1),
                xtream.map(|x| x.2),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_playlists(&self) -> rusqlite::Result<Vec<Playlist>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, source_type, source_url, xtream_base, created_at FROM playlists ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Playlist {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    source_type: r.get(2)?,
                    source_url: r.get(3)?,
                    xtream_base: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_playlist(&self, id: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM playlists WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- channels ----------

    /// Batch-import parsed channels: resolves groups by name (creating
    /// new ones), inserts in BATCH-sized transactions so a huge import
    /// doesn't build one giant WAL.
    pub fn import_channels(
        &self,
        playlist_id: i64,
        channels: &[ParsedChannel],
    ) -> rusqlite::Result<ImportStats> {
        let conn = self.conn.lock().unwrap();

        // group name → id, seeded with existing groups
        let mut groups: Vec<(i64, String)> = {
            let mut stmt = conn.prepare("SELECT id, name FROM groups WHERE playlist_id = ?1")?;
            let rows = stmt
                .query_map(params![playlist_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let groups_before = groups.len();

        // Each batch gets its own transaction and prepared statement, so a
        // statement never outlives (or borrows across) the transaction it
        // was prepared from — commit() consumes the transaction.
        for batch in channels.chunks(BATCH) {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO channels (playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'live')",
                )?;
                for ch in batch {
                    let gid = match &ch.group {
                        Some(name) => {
                            if let Some((id, _)) = groups.iter().find(|(_, n)| n == name) {
                                Some(*id)
                            } else {
                                tx.execute(
                                    "INSERT INTO groups (playlist_id, name) VALUES (?1, ?2)",
                                    params![playlist_id, name],
                                )?;
                                let id = tx.last_insert_rowid();
                                groups.push((id, name.to_string()));
                                Some(id)
                            }
                        }
                        None => None,
                    };
                    stmt.execute(params![
                        playlist_id,
                        gid,
                        ch.name,
                        ch.url,
                        ch.logo_url,
                        ch.tvg_id,
                        ch.tvg_chno,
                    ])?;
                }
            }
            tx.commit()?;
        }

        Ok(ImportStats {
            channels: channels.len(),
            groups: groups.len() - groups_before,
        })
    }

    pub fn list_groups(&self, playlist_id: i64) -> rusqlite::Result<Vec<Group>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id, playlist_id, name FROM groups WHERE playlist_id = ?1 ORDER BY name")?;
        let rows = stmt
            .query_map(params![playlist_id], |r| {
                Ok(Group {
                    id: r.get(0)?,
                    playlist_id: r.get(1)?,
                    name: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// All groups across playlists (TV server navigation).
    pub fn groups_all(&self) -> rusqlite::Result<Vec<Group>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, playlist_id, name FROM groups ORDER BY name")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Group {
                    id: r.get(0)?,
                    playlist_id: r.get(1)?,
                    name: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn channels_by_group(&self, group_id: i64) -> rusqlite::Result<Vec<Channel>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind
             FROM channels WHERE group_id = ?1 ORDER BY COALESCE(tvg_chno, 999999), name",
        )?;
        let rows = stmt
            .query_map(params![group_id], |r| row_to_channel(r))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn channels_all(&self, limit: i64) -> rusqlite::Result<Vec<Channel>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind
             FROM channels ORDER BY name LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit], |r| row_to_channel(r))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Case-insensitive substring search with % and _ escaped.
    pub fn search_channels(
        &self,
        query: &str,
        playlist_id: Option<i64>,
        limit: i64,
    ) -> rusqlite::Result<Vec<Channel>> {
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{}%", escaped);
        let conn = self.conn.lock().unwrap();
        // Bind the collect result to a local so the MappedRows temporary
        // drops before `stmt` (otherwise the borrow outlives the statement).
        let rows = match playlist_id {
            Some(pid) => {
                let mut stmt = conn.prepare(
                    "SELECT id, playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind
                     FROM channels WHERE playlist_id = ?1 AND name LIKE ?2 ESCAPE '\\'
                     ORDER BY name LIMIT ?3",
                )?;
                let rows = stmt
                    .query_map(params![pid, pattern, limit], |r| row_to_channel(r))?
                    .collect::<Result<Vec<_>, _>>()?;
                rows
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id, playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind
                     FROM channels WHERE name LIKE ?1 ESCAPE '\\'
                     ORDER BY name LIMIT ?2",
                )?;
                let rows = stmt
                    .query_map(params![pattern, limit], |r| row_to_channel(r))?
                    .collect::<Result<Vec<_>, _>>()?;
                rows
            }
        };
        Ok(rows)
    }

    pub fn get_channel(&self, id: i64) -> rusqlite::Result<Option<Channel>> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT id, playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind
                 FROM channels WHERE id = ?1",
                params![id],
                row_to_channel,
            )
            .optional()?;
        Ok(row)
    }

    pub fn channel_count(&self) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM channels", [], |r| r.get(0))
            .map_err(Into::into)
    }
}

fn row_to_channel(r: &rusqlite::Row<'_>) -> rusqlite::Result<Channel> {
    Ok(Channel {
        id: r.get(0)?,
        playlist_id: r.get(1)?,
        group_id: r.get(2)?,
        name: r.get(3)?,
        url: r.get(4)?,
        logo_url: r.get(5)?,
        tvg_id: r.get(6)?,
        tvg_chno: r.get(7)?,
        kind: r.get(8)?,
    })
}
