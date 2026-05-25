use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use crate::{OsmosError, Result};

/// SQLite-backed metadata store + content-addressable blob directory.
///
/// Layout on disk:
/// ```
/// <repo_root>/.osmos/
///   meta.db      — SQLite (repos, commits, trees, branches, peers)
///   blobs/ab/cd… — blob files named by BLAKE3 hex prefix-sharded
/// ```
pub struct Store {
    pub(crate) conn: Connection,
    blobs_dir: PathBuf,
}

impl Store {
    /// Opens or creates the store at `<root>/.osmos/`.
    pub fn open(root: &Path) -> Result<Self> {
        let osmos_dir = root.join(".osmos");
        std::fs::create_dir_all(&osmos_dir)?;

        let blobs_dir = osmos_dir.join("blobs");
        std::fs::create_dir_all(&blobs_dir)?;

        let db_path = osmos_dir.join("meta.db");
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        let store = Self { conn, blobs_dir };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let version: i32 = self.conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);

        // v0 → v1: base schema
        if version < 1 {
            self.conn.execute_batch("
                CREATE TABLE IF NOT EXISTS repositories (
                    id          TEXT PRIMARY KEY,
                    name        TEXT NOT NULL,
                    root_path   TEXT NOT NULL UNIQUE,
                    created_at  TEXT NOT NULL,
                    mode        TEXT NOT NULL DEFAULT 'client'
                );

                CREATE TABLE IF NOT EXISTS commits (
                    id          TEXT PRIMARY KEY,
                    repo_id     TEXT NOT NULL REFERENCES repositories(id),
                    parent_id   TEXT,
                    parent_ids  TEXT NOT NULL DEFAULT '[]',
                    message     TEXT NOT NULL,
                    tree_hash   TEXT NOT NULL,
                    author      TEXT NOT NULL,
                    created_at  TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS tree_entries (
                    tree_hash   TEXT NOT NULL,
                    name        TEXT NOT NULL,
                    blob_hash   TEXT NOT NULL,
                    size_bytes  INTEGER NOT NULL,
                    PRIMARY KEY (tree_hash, name)
                );

                CREATE TABLE IF NOT EXISTS branches (
                    id             TEXT PRIMARY KEY,
                    repo_id        TEXT NOT NULL REFERENCES repositories(id),
                    name           TEXT NOT NULL,
                    head_commit_id TEXT,
                    created_at     TEXT NOT NULL,
                    UNIQUE(repo_id, name)
                );

                CREATE TABLE IF NOT EXISTS peers (
                    id           TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    address      TEXT NOT NULL,
                    last_seen    TEXT NOT NULL,
                    mode         TEXT NOT NULL DEFAULT 'client'
                );

                PRAGMA user_version = 1;
            ")?;
        }

        // v1 → v2: add parent_ids + branches to pre-existing databases
        if version == 1 {
            // ALTER TABLE doesn't support IF NOT EXISTS — ignore error if column exists.
            let _ = self.conn.execute_batch(
                "ALTER TABLE commits ADD COLUMN parent_ids TEXT NOT NULL DEFAULT '[]';"
            );
            // Backfill parent_ids from the legacy parent_id column.
            self.conn.execute_batch("
                UPDATE commits
                SET parent_ids = json_array(parent_id)
                WHERE parent_id IS NOT NULL AND parent_ids = '[]';
            ")?;
            self.conn.execute_batch("
                CREATE TABLE IF NOT EXISTS branches (
                    id             TEXT PRIMARY KEY,
                    repo_id        TEXT NOT NULL REFERENCES repositories(id),
                    name           TEXT NOT NULL,
                    head_commit_id TEXT,
                    created_at     TEXT NOT NULL,
                    UNIQUE(repo_id, name)
                );
                PRAGMA user_version = 2;
            ")?;
        }

        Ok(())
    }

    // MARK: - Blob store

    pub fn write_blob(&self, data: &[u8]) -> Result<String> {
        let hash = crate::hash::blake3_hex(data);
        let blob_path = self.blob_path(&hash);
        if !blob_path.exists() {
            std::fs::create_dir_all(blob_path.parent().unwrap())?;
            std::fs::write(&blob_path, data)?;
        }
        Ok(hash)
    }

    /// Streams `src` into the blob store without loading it entirely into RAM.
    /// Returns `(blake3_hex, file_size_bytes)`.
    pub fn write_blob_streaming(&self, src: &Path) -> Result<(String, u64)> {
        let hash = crate::hash::blake3_file(src)?;
        let size = src.metadata()?.len();
        let blob_path = self.blob_path(&hash);
        if !blob_path.exists() {
            std::fs::create_dir_all(blob_path.parent().unwrap())?;
            std::fs::copy(src, &blob_path)?;
        }
        Ok((hash, size))
    }

    pub fn read_blob(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.blob_path(hash);
        std::fs::read(&path).map_err(|_| OsmosError::BlobNotFound { hash: hash.to_owned() })
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        let (prefix, rest) = hash.split_at(2);
        self.blobs_dir.join(prefix).join(rest)
    }

    // MARK: - Repository

    pub fn insert_repo(&self, repo: &crate::repo::Repository) -> Result<()> {
        self.conn.execute(
            "INSERT INTO repositories (id, name, root_path, created_at, mode)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                repo.id.to_string(),
                repo.name,
                repo.root_path.to_string_lossy().to_string(),
                repo.created_at.to_rfc3339(),
                repo.mode.as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn list_repos(&self) -> Result<Vec<crate::repo::Repository>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, root_path, created_at, mode FROM repositories ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut repos = Vec::new();
        for row in rows {
            let (id, name, path, created_at, mode) = row?;
            repos.push(crate::repo::Repository {
                id: Uuid::parse_str(&id).unwrap_or_default(),
                name,
                root_path: PathBuf::from(path),
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Utc),
                mode: crate::repo::RepoMode::from_str(&mode),
            });
        }
        Ok(repos)
    }

    pub fn delete_repo_by_path(&self, root_path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM commits WHERE repo_id IN (SELECT id FROM repositories WHERE root_path = ?1)",
            params![root_path],
        )?;
        self.conn.execute(
            "DELETE FROM branches WHERE repo_id IN (SELECT id FROM repositories WHERE root_path = ?1)",
            params![root_path],
        )?;
        self.conn.execute(
            "DELETE FROM repositories WHERE root_path = ?1",
            params![root_path],
        )?;
        Ok(())
    }

    // MARK: - Commits

    pub fn insert_commit(&self, commit: &crate::commit::Commit) -> Result<()> {
        let parent_ids_json = serde_json::to_string(&commit.parent_ids)?;
        let first_parent = commit.parent_ids.first().map(|u| u.to_string());
        self.conn.execute(
            "INSERT INTO commits (id, repo_id, parent_id, parent_ids, message, tree_hash, author, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                commit.id.to_string(),
                commit.repo_id.to_string(),
                first_parent,
                parent_ids_json,
                commit.message,
                commit.tree_hash,
                commit.author,
                commit.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn get_commit(&self, id: Uuid) -> Result<Option<crate::commit::Commit>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repo_id, parent_ids, message, tree_hash, author, created_at
             FROM commits WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        if let Some(row) = rows.next() {
            let (id, rid, parent_ids_str, message, tree_hash, author, created_at) = row?;
            Ok(Some(crate::commit::Commit {
                id:         Uuid::parse_str(&id).unwrap_or_default(),
                repo_id:    Uuid::parse_str(&rid).unwrap_or_default(),
                parent_ids: serde_json::from_str(&parent_ids_str).unwrap_or_default(),
                message,
                tree_hash,
                author,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Utc),
            }))
        } else {
            Ok(None)
        }
    }

    /// Returns all commits for a repo, newest first (flat list — use `list_commits_from` for DAG walk).
    pub fn list_commits(&self, repo_id: Uuid) -> Result<Vec<crate::commit::Commit>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repo_id, parent_ids, message, tree_hash, author, created_at
             FROM commits WHERE repo_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![repo_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;

        let mut commits = Vec::new();
        for row in rows {
            let (id, rid, parent_ids_str, message, tree_hash, author, created_at) = row?;
            commits.push(crate::commit::Commit {
                id:         Uuid::parse_str(&id).unwrap_or_default(),
                repo_id:    Uuid::parse_str(&rid).unwrap_or_default(),
                parent_ids: serde_json::from_str(&parent_ids_str).unwrap_or_default(),
                message,
                tree_hash,
                author,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Utc),
            });
        }
        Ok(commits)
    }

    /// DAG walk from `head_id`, returns all reachable commits newest-first.
    pub fn list_commits_from(&self, head_id: Uuid) -> Result<Vec<crate::commit::Commit>> {
        let mut result   = Vec::new();
        let mut to_visit = vec![head_id];
        let mut visited  = std::collections::HashSet::new();

        while let Some(id) = to_visit.pop() {
            if !visited.insert(id) { continue; }
            if let Some(commit) = self.get_commit(id)? {
                for &pid in &commit.parent_ids {
                    to_visit.push(pid);
                }
                result.push(commit);
            }
        }

        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    // MARK: - Tree entries

    /// Returns `{ relative_path → (blob_hash, size_bytes) }` for a tree hash.
    pub fn get_tree_map(&self, tree_hash: &str) -> Result<HashMap<String, (String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, blob_hash, size_bytes FROM tree_entries WHERE tree_hash = ?1",
        )?;
        let rows = stmt.query_map(params![tree_hash], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (name, blob_hash, size) = row?;
            map.insert(name, (blob_hash, size));
        }
        Ok(map)
    }

    // MARK: - Branches

    pub fn insert_branch(&self, branch: &crate::branch::Branch) -> Result<()> {
        self.conn.execute(
            "INSERT INTO branches (id, repo_id, name, head_commit_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                branch.id.to_string(),
                branch.repo_id.to_string(),
                branch.name,
                branch.head_commit_id.map(|u| u.to_string()),
                branch.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn get_branch(&self, repo_id: Uuid, name: &str) -> Result<Option<crate::branch::Branch>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repo_id, name, head_commit_id, created_at
             FROM branches WHERE repo_id = ?1 AND name = ?2",
        )?;
        let mut rows = stmt.query_map(params![repo_id.to_string(), name], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        if let Some(row) = rows.next() {
            let (id, rid, name, head_str, created_at) = row?;
            Ok(Some(crate::branch::Branch {
                id:             Uuid::parse_str(&id).unwrap_or_default(),
                repo_id:        Uuid::parse_str(&rid).unwrap_or_default(),
                name,
                head_commit_id: head_str.and_then(|s| Uuid::parse_str(&s).ok()),
                created_at:     chrono::DateTime::parse_from_rfc3339(&created_at)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Utc),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_branches(&self, repo_id: Uuid) -> Result<Vec<crate::branch::Branch>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repo_id, name, head_commit_id, created_at
             FROM branches WHERE repo_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![repo_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut branches = Vec::new();
        for row in rows {
            let (id, rid, name, head_str, created_at) = row?;
            branches.push(crate::branch::Branch {
                id:             Uuid::parse_str(&id).unwrap_or_default(),
                repo_id:        Uuid::parse_str(&rid).unwrap_or_default(),
                name,
                head_commit_id: head_str.and_then(|s| Uuid::parse_str(&s).ok()),
                created_at:     chrono::DateTime::parse_from_rfc3339(&created_at)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Utc),
            });
        }
        Ok(branches)
    }

    pub fn update_branch_head(&self, branch_id: Uuid, head_commit_id: Option<Uuid>) -> Result<()> {
        self.conn.execute(
            "UPDATE branches SET head_commit_id = ?1 WHERE id = ?2",
            params![
                head_commit_id.map(|u| u.to_string()),
                branch_id.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn delete_branch(&self, repo_id: Uuid, name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM branches WHERE repo_id = ?1 AND name = ?2",
            params![repo_id.to_string(), name],
        )?;
        Ok(())
    }
}
