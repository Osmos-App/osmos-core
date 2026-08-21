use crate::{store::Store, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// A point-in-time snapshot of a repository's working tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub id: Uuid,
    pub repo_id: Uuid,
    /// Empty for the root commit; two entries for a merge commit.
    pub parent_ids: Vec<Uuid>,
    pub message: String,
    /// BLAKE3 hex hash identifying the tree snapshot stored in `tree_entries`.
    pub tree_hash: String,
    pub author: String,
    pub created_at: DateTime<Utc>,
}

/// A single file entry within a commit's tree snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeEntry {
    pub name: String,
    pub blob_hash: String,
    pub size_bytes: u64,
}

impl Commit {
    /// Snapshots the working tree, writes all changed blobs, and persists the commit.
    ///
    /// `repo_root` must point to the directory containing `.osmos/`.
    pub fn create(
        repo_root: &Path,
        repo_id: Uuid,
        parent_ids: Vec<Uuid>,
        message: &str,
        author: &str,
    ) -> Result<Self> {
        let store = Store::open(repo_root)?;
        let mut entries: Vec<TreeEntry> = Vec::new();
        let mut tree_hasher = blake3::Hasher::new();

        // Walk and hash every file, skip .osmos/.
        // write_blob_streaming reads in 64 KiB chunks — no whole-file allocation.
        for entry in walkdir::WalkDir::new(repo_root)
            .min_depth(1)
            .into_iter()
            .filter_entry(|e| e.file_name() != ".osmos")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let abs = entry.path();
            let (blob_hash, size_bytes) = store.write_blob_streaming(abs)?;
            let rel = abs
                .strip_prefix(repo_root)
                .unwrap_or(abs)
                .to_string_lossy()
                .to_string();

            tree_hasher.update(rel.as_bytes());
            tree_hasher.update(blob_hash.as_bytes());

            entries.push(TreeEntry {
                name: rel,
                blob_hash,
                size_bytes,
            });
        }

        let tree_hash = tree_hasher.finalize().to_hex().to_string();

        // Persist tree entries.
        for e in &entries {
            store.conn.execute(
                "INSERT OR REPLACE INTO tree_entries (tree_hash, name, blob_hash, size_bytes)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![tree_hash, e.name, e.blob_hash, e.size_bytes],
            )?;
        }

        let commit = Commit {
            id: Uuid::new_v4(),
            repo_id,
            parent_ids,
            message: message.to_owned(),
            tree_hash,
            author: author.to_owned(),
            created_at: Utc::now(),
        };

        store.insert_commit(&commit)?;
        tracing::info!(commit_id = %commit.id, message, "commit created");
        Ok(commit)
    }
}
