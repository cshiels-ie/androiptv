//! SQLite persistence. One connection behind a mutex (rusqlite's
//! Connection is Send but not Sync); heavy imports run inside
//! `spawn_blocking` so the UI never stalls.
//!
//! The `kind` column discriminates live/VOD/series rows so all three
//! share the channels table; series episodes live in their own table.

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
    pub kind: String,
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
    pub remote_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Episode {
    pub id: i64,
    pub channel_id: i64,
    pub season: i64,
    pub episode_num: i64,
    pub title: String,
    pub url: String,
    pub logo_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportStats {
    pub channels: usize,
    pub groups: usize,
    pub vod: usize,
    pub series: usize,
}

/// One row ready for the unified importer (live / vod / series).
pub struct NewChannel {
    pub name: String,
    pub url: String,
    pub logo_url: Option<String>,
    pub tvg_id: Option<String>,
    pub tvg_chno: Option<i64>,
    pub kind: String,
    pub remote_id: Option<String>,
    pub group: Option<String>,
}

impl From<ParsedChannel> for NewChannel {
    fn from(ch: ParsedChannel) -> Self {
        NewChannel {
            name: ch.name,
            url: ch.url,
            logo_url: ch.logo_url,
            tvg_id: ch.tvg_id,
            tvg_chno: ch.tvg_chno,
            kind: "live".into(),
            remote_id: None,
            group: ch.group,
        }
    }
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
  kind TEXT NOT NULL DEFAULT 'live',
  UNIQUE(playlist_id, kind, name)
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
  kind TEXT NOT NULL DEFAULT 'live',
  remote_id TEXT
);
CREATE TABLE IF NOT EXISTS episodes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  season INTEGER NOT NULL,
  episode_num INTEGER NOT NULL,
  title TEXT NOT NULL,
  url TEXT NOT NULL,
  logo_url TEXT,
  UNIQUE(channel_id, season, episode_num)
);
CREATE TABLE IF NOT EXISTS series_meta (
  channel_id INTEGER PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
  fetched_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ch_playlist ON channels(playlist_id);
CREATE INDEX IF NOT EXISTS idx_ch_group ON channels(group_id);
CREATE INDEX IF NOT EXISTS idx_ch_name ON channels(name COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_ep_channel ON episodes(channel_id);
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
        conn.execute_batch(SCHEMA)?;
        migrate_conn(&conn)
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

    /// Xtream base/user/pass for a playlist, if it was imported from an
    /// Xtream panel (used to rebuild VOD/series URLs on demand).
    pub fn playlist_xtream_creds(
        &self,
        playlist_id: i64,
    ) -> rusqlite::Result<Option<(String, String, String)>> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT xtream_base, xtream_user, xtream_pass FROM playlists WHERE id = ?1",
                params![playlist_id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.and_then(|(b, u, p)| match (b, u, p) {
            (Some(b), Some(u), Some(p)) => Some((b, u, p)),
            _ => None,
        }))
    }

    // ---------- channels ----------

    /// Batch-import M3U channels (kind 'live').
    pub fn import_channels(
        &self,
        playlist_id: i64,
        channels: &[ParsedChannel],
    ) -> rusqlite::Result<ImportStats> {
        let items: Vec<NewChannel> = channels.iter().cloned().map(NewChannel::from).collect();
        self.import_items(playlist_id, &items)
    }

    /// Batch-import kind-tagged rows: resolves groups by (kind, name)
    /// — VOD and series may share category names with live channels —
    /// and inserts in BATCH-sized transactions so a huge import doesn't
    /// build one giant WAL.
    pub fn import_items(
        &self,
        playlist_id: i64,
        items: &[NewChannel],
    ) -> rusqlite::Result<ImportStats> {
        let conn = self.conn.lock().unwrap();

        // (id, kind, name) -> id, seeded with existing groups
        let mut groups: Vec<(i64, String, String)> = {
            let mut stmt = conn.prepare("SELECT id, kind, name FROM groups WHERE playlist_id = ?1")?;
            let rows = stmt
                .query_map(params![playlist_id], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let groups_before = groups.len();

        // Each batch gets its own transaction and prepared statement, so a
        // statement never outlives (or borrows across) the transaction it
        // was prepared from — commit() consumes the transaction.
        for batch in items.chunks(BATCH) {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO channels (playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind, remote_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )?;
                for ch in batch {
                    let gid = match &ch.group {
                        Some(name) => {
                            if let Some((id, _, _)) = groups
                                .iter()
                                .find(|(_, k, n)| k == &ch.kind && n == name)
                            {
                                Some(*id)
                            } else {
                                tx.execute(
                                    "INSERT INTO groups (playlist_id, kind, name) VALUES (?1, ?2, ?3)",
                                    params![playlist_id, ch.kind, name],
                                )?;
                                let id = tx.last_insert_rowid();
                                groups.push((id, ch.kind.clone(), name.clone()));
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
                        ch.kind,
                        ch.remote_id,
                    ])?;
                }
            }
            tx.commit()?;
        }

        Ok(ImportStats {
            channels: items.iter().filter(|i| i.kind == "live").count(),
            vod: items.iter().filter(|i| i.kind == "vod").count(),
            series: items.iter().filter(|i| i.kind == "series").count(),
            groups: groups.len() - groups_before,
        })
    }

    pub fn list_groups(&self, playlist_id: i64, kind: Option<&str>) -> rusqlite::Result<Vec<Group>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, playlist_id, name, kind FROM groups
             WHERE playlist_id = ?1 AND (?2 IS NULL OR kind = ?2) ORDER BY name",
        )?;
        let rows = stmt
            .query_map(params![playlist_id, kind], |r| row_to_group(r))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// All groups of one kind across playlists (TV server navigation).
    pub fn groups_all(&self, kind: &str) -> rusqlite::Result<Vec<Group>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id, playlist_id, name, kind FROM groups WHERE kind = ?1 ORDER BY name")?;
        let rows = stmt
            .query_map(params![kind], |r| row_to_group(r))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn channels_by_group(&self, group_id: i64) -> rusqlite::Result<Vec<Channel>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind, remote_id
             FROM channels WHERE group_id = ?1 ORDER BY COALESCE(tvg_chno, 999999), name",
        )?;
        let rows = stmt
            .query_map(params![group_id], |r| row_to_channel(r))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn channels_all(&self, limit: i64, kind: &str) -> rusqlite::Result<Vec<Channel>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind, remote_id
             FROM channels WHERE kind = ?1 ORDER BY name LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![kind, limit], |r| row_to_channel(r))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Case-insensitive substring search with % and _ escaped.
    pub fn search_channels(
        &self,
        query: &str,
        playlist_id: Option<i64>,
        kind: Option<&str>,
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
                    "SELECT id, playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind, remote_id
                     FROM channels WHERE playlist_id = ?1 AND name LIKE ?2 ESCAPE '\\'
                     AND (?3 IS NULL OR kind = ?3)
                     ORDER BY name LIMIT ?4",
                )?;
                let rows = stmt
                    .query_map(params![pid, pattern, kind, limit], |r| row_to_channel(r))?
                    .collect::<Result<Vec<_>, _>>()?;
                rows
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id, playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind, remote_id
                     FROM channels WHERE name LIKE ?1 ESCAPE '\\'
                     AND (?2 IS NULL OR kind = ?2)
                     ORDER BY name LIMIT ?3",
                )?;
                let rows = stmt
                    .query_map(params![pattern, kind, limit], |r| row_to_channel(r))?
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
                "SELECT id, playlist_id, group_id, name, url, logo_url, tvg_id, tvg_chno, kind, remote_id
                 FROM channels WHERE id = ?1",
                params![id],
                row_to_channel,
            )
            .optional()?;
        Ok(row)
    }

    /// Live-only count (what the TV page can browse).
    pub fn channel_count(&self) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM channels WHERE kind = 'live'", [], |r| r.get(0))
            .map_err(Into::into)
    }

    // ---------- series episodes ----------

    /// Episodes cached for a series channel. `None` = never fetched yet
    /// (the lazy `get_series_info` path must run); `Some(vec)` = cached
    /// result, possibly empty (a fetched series with no episodes — the
    /// series_meta marker row distinguishes it from never-fetched).
    pub fn episodes_for_channel(&self, channel_id: i64) -> rusqlite::Result<Option<Vec<Episode>>> {
        let conn = self.conn.lock().unwrap();
        let fetched: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM series_meta WHERE channel_id = ?1)",
            params![channel_id],
            |r| r.get(0),
        )?;
        if !fetched {
            return Ok(None);
        }
        let mut stmt = conn.prepare(
            "SELECT id, channel_id, season, episode_num, title, url, logo_url
             FROM episodes WHERE channel_id = ?1 ORDER BY season, episode_num",
        )?;
        let rows = stmt
            .query_map(params![channel_id], |r| row_to_episode(r))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(rows))
    }

    /// Replace the whole episode cache for a channel in one transaction.
    pub fn replace_episodes(&self, channel_id: i64, eps: &[Episode]) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        {
            tx.execute("DELETE FROM episodes WHERE channel_id = ?1", params![channel_id])?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO episodes (channel_id, season, episode_num, title, url, logo_url)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )?;
                for ep in eps {
                    stmt.execute(params![ep.channel_id, ep.season, ep.episode_num, ep.title, ep.url, ep.logo_url])?;
                }
            }
            tx.execute(
                "INSERT OR REPLACE INTO series_meta (channel_id, fetched_at) VALUES (?1, strftime('%s','now'))",
                params![channel_id],
            )?;
        }
        tx.commit()
    }

    pub fn get_episode(&self, id: i64) -> rusqlite::Result<Option<Episode>> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT id, channel_id, season, episode_num, title, url, logo_url
                 FROM episodes WHERE id = ?1",
                params![id],
                row_to_episode,
            )
            .optional()?;
        Ok(row)
    }

    // ---------- settings ----------

    /// Reads a setting (e.g. the server host/port override), or None when
    /// the key was never written.
    pub fn get_setting(&self, key: &str) -> rusqlite::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let value = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value)
    }

    /// Upserts a setting.
    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
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
        remote_id: r.get(9)?,
    })
}

fn row_to_group(r: &rusqlite::Row<'_>) -> rusqlite::Result<Group> {
    Ok(Group {
        id: r.get(0)?,
        playlist_id: r.get(1)?,
        name: r.get(2)?,
        kind: r.get(3)?,
    })
}

fn row_to_episode(r: &rusqlite::Row<'_>) -> rusqlite::Result<Episode> {
    Ok(Episode {
        id: r.get(0)?,
        channel_id: r.get(1)?,
        season: r.get(2)?,
        episode_num: r.get(3)?,
        title: r.get(4)?,
        url: r.get(5)?,
        logo_url: r.get(6)?,
    })
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
    Ok(names.collect::<Result<Vec<_>, _>>()?.iter().any(|n| n == column))
}

/// Bring pre-VOD databases up to the current schema. Idempotent (each
/// step is guarded by a column check). The groups rebuild is the risky
/// part: the old UNIQUE(playlist_id, name) is an autoindex, so the table
/// must be recreated — and with `PRAGMA foreign_keys=ON` (which
/// init_schema sets), `DROP TABLE groups` would fire `ON DELETE SET NULL`
/// and wipe every channels.group_id, so foreign keys are disabled for
/// the DDL and re-enabled after.
fn migrate_conn(conn: &Connection) -> rusqlite::Result<()> {
    if !column_exists(conn, "channels", "remote_id")? {
        conn.execute("ALTER TABLE channels ADD COLUMN remote_id TEXT", [])?;
    }
    if !column_exists(conn, "groups", "kind")? {
        let result = conn.execute_batch(
            "PRAGMA foreign_keys=OFF;
             BEGIN;
             CREATE TABLE groups_v2 (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
               name TEXT NOT NULL,
               kind TEXT NOT NULL DEFAULT 'live',
               UNIQUE(playlist_id, kind, name)
             );
             INSERT INTO groups_v2 (id, playlist_id, name) SELECT id, playlist_id, name FROM groups;
             DROP TABLE groups;
             ALTER TABLE groups_v2 RENAME TO groups;
             COMMIT;
             PRAGMA foreign_keys=ON;",
        );
        if let Err(e) = result {
            // Best-effort restore so the connection stays consistent even
            // on failure (init then returns Err and the app reports it).
            let _ = conn.execute_batch("ROLLBACK; PRAGMA foreign_keys=ON;");
            return Err(e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The schema as released before VOD support (no remote_id, no
    /// groups.kind, old UNIQUE(playlist_id, name)).
    const LEGACY_SCHEMA: &str = r#"
CREATE TABLE playlists (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  source_type TEXT NOT NULL,
  source_url TEXT NOT NULL,
  xtream_base TEXT,
  xtream_user TEXT,
  xtream_pass TEXT,
  created_at INTEGER NOT NULL
);
CREATE TABLE groups (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  UNIQUE(playlist_id, name)
);
CREATE TABLE channels (
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
"#;

    fn xtream_playlist(db: &Db) -> i64 {
        db.create_playlist("t", "xtream", "http://x/player_api.php", Some(("http://x", "u", "p")))
            .unwrap()
    }

    fn series_item(name: &str, remote_id: &str) -> NewChannel {
        NewChannel {
            name: name.into(),
            url: String::new(),
            logo_url: None,
            tvg_id: None,
            tvg_chno: None,
            kind: "series".into(),
            remote_id: Some(remote_id.into()),
            group: Some("Drama".into()),
        }
    }

    #[test]
    fn legacy_schema_migrates() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(LEGACY_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO playlists (name, source_type, source_url, created_at) VALUES ('t', 'm3u', 'x', 0)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO groups (playlist_id, name) VALUES (1, 'News')", []).unwrap();
        conn.execute(
            "INSERT INTO channels (playlist_id, group_id, name, url, kind) VALUES (1, 1, 'cnn', 'http://x', 'live')",
            [],
        )
        .unwrap();

        migrate_conn(&conn).unwrap();

        // New columns exist.
        assert!(column_exists(&conn, "channels", "remote_id").unwrap());
        assert!(column_exists(&conn, "groups", "kind").unwrap());

        // Same-named groups of different kinds can now coexist.
        conn.execute("INSERT INTO groups (playlist_id, kind, name) VALUES (1, 'vod', 'News')", []).unwrap();
        // ...and the original group survives as kind 'live' (dup insert fails).
        assert!(conn
            .execute("INSERT INTO groups (playlist_id, kind, name) VALUES (1, 'live', 'News')", [])
            .is_err());

        // The pre-existing channel's group link is intact.
        let gid: i64 = conn.query_row("SELECT group_id FROM channels WHERE id = 1", [], |r| r.get(0)).unwrap();
        let name: String = conn.query_row("SELECT name FROM groups WHERE id = ?1", params![gid], |r| r.get(0)).unwrap();
        assert_eq!(name, "News");
        // Its kind defaulted to 'live'.
        let kind: String = conn.query_row("SELECT kind FROM groups WHERE id = ?1", params![gid], |r| r.get(0)).unwrap();
        assert_eq!(kind, "live");
    }

    #[test]
    fn import_items_mixed_kinds() {
        let db = Db::open_in_memory().unwrap();
        let pid = xtream_playlist(&db);
        let items = vec![
            NewChannel {
                name: "BBC1".into(),
                url: "http://x/live/u/p/1.ts".into(),
                logo_url: None,
                tvg_id: None,
                tvg_chno: None,
                kind: "live".into(),
                remote_id: None,
                group: Some("News".into()),
            },
            NewChannel {
                name: "A Movie".into(),
                url: "http://x/movie/u/p/2.mp4".into(),
                logo_url: None,
                tvg_id: None,
                tvg_chno: None,
                kind: "vod".into(),
                remote_id: Some("2".into()),
                group: Some("News".into()),
            },
            series_item("A Show", "3"),
        ];

        let stats = db.import_items(pid, &items).unwrap();
        assert_eq!(stats.channels, 1);
        assert_eq!(stats.vod, 1);
        assert_eq!(stats.series, 1);
        assert_eq!(stats.groups, 3); // one group per kind, same name

        assert_eq!(db.list_groups(pid, Some("live")).unwrap().len(), 1);
        assert_eq!(db.list_groups(pid, Some("vod")).unwrap().len(), 1);
        assert_eq!(db.list_groups(pid, Some("series")).unwrap().len(), 1);
        assert_eq!(db.list_groups(pid, None).unwrap().len(), 3);

        // Kind-filtered search; remote_id round-trips.
        let series = db.search_channels("A Show", Some(pid), Some("series"), 10).unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].remote_id.as_deref(), Some("3"));
        assert!(db.search_channels("A Movie", Some(pid), Some("live"), 10).unwrap().is_empty());
        assert!(db.search_channels("A Movie", Some(pid), None, 10).unwrap().len() == 1);
    }

    #[test]
    fn episodes_replace_and_cascade() {
        let db = Db::open_in_memory().unwrap();
        let pid = xtream_playlist(&db);
        db.import_items(pid, &[series_item("Show", "3")]).unwrap();
        let ch = db.search_channels("Show", Some(pid), Some("series"), 5).unwrap().remove(0);

        // Not cached yet.
        assert!(db.episodes_for_channel(ch.id).unwrap().is_none());

        let mk = |season: i64, num: i64| Episode {
            id: 0,
            channel_id: ch.id,
            season,
            episode_num: num,
            title: format!("S{season}E{num}"),
            url: "http://x/series/u/p/3/1/1.mp4".into(),
            logo_url: None,
        };
        db.replace_episodes(ch.id, &[mk(1, 1), mk(1, 2)]).unwrap();
        let cached = db.episodes_for_channel(ch.id).unwrap().unwrap();
        assert_eq!(cached.len(), 2);
        assert_eq!(cached[0].episode_num, 1);
        assert!(db.get_episode(cached[0].id).unwrap().is_some());

        // Replacement is wholesale.
        db.replace_episodes(ch.id, &[mk(2, 1)]).unwrap();
        assert_eq!(db.episodes_for_channel(ch.id).unwrap().unwrap().len(), 1);

        // An empty fetch still marks the cache (no re-fetch loop).
        db.replace_episodes(ch.id, &[]).unwrap();
        assert_eq!(db.episodes_for_channel(ch.id).unwrap().unwrap().len(), 0);

        // Deleting the playlist cascades channels → episodes → series_meta.
        db.delete_playlist(pid).unwrap();
        assert!(db.episodes_for_channel(ch.id).unwrap().is_none());
        assert!(db.get_episode(1).unwrap().is_none());
    }

    #[test]
    fn playlist_creds_only_for_xtream() {
        let db = Db::open_in_memory().unwrap();
        let pid = xtream_playlist(&db);
        assert_eq!(
            db.playlist_xtream_creds(pid).unwrap(),
            Some(("http://x".to_string(), "u".to_string(), "p".to_string()))
        );
        let m3u = db.create_playlist("m", "m3u", "http://m/pl.m3u", None).unwrap();
        assert!(db.playlist_xtream_creds(m3u).unwrap().is_none());
    }

    #[test]
    fn settings_upsert_and_read() {
        let db = Db::open_in_memory().unwrap();
        // Unset key reads as None.
        assert!(db.get_setting("server_port").unwrap().is_none());
        // Insert, overwrite, delete-free read.
        db.set_setting("server_port", "8080").unwrap();
        assert_eq!(db.get_setting("server_port").unwrap(), Some("8080".into()));
        db.set_setting("server_port", "9090").unwrap();
        assert_eq!(db.get_setting("server_port").unwrap(), Some("9090".into()));
        // Independent keys don't collide.
        db.set_setting("server_ip_override", "192.168.1.50").unwrap();
        assert_eq!(db.get_setting("server_port").unwrap(), Some("9090".into()));
    }
}
