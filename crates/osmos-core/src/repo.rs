use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use crate::{OsmosError, Result, store::Store, branch::Branch, commit::Commit};

/// Whether this node acts as a sync hub or a connecting client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RepoMode {
    Hub,
    Client,
}

impl RepoMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RepoMode::Hub    => "hub",
            RepoMode::Client => "client",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "hub" => RepoMode::Hub,
            _     => RepoMode::Client,
        }
    }
}

/// A tracked project directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id:         Uuid,
    pub name:       String,
    pub root_path:  PathBuf,
    pub created_at: DateTime<Utc>,
    pub mode:       RepoMode,
}

impl Repository {
    /// Initializes a new Osmos repository at `path`, persists it in the store,
    /// and creates the default `main` branch.
    pub fn init(path: &Path, name: &str, mode: RepoMode) -> Result<Self> {
        let osmos_dir = path.join(".osmos");
        if osmos_dir.exists() {
            return Err(OsmosError::RepoExists(path.display().to_string()));
        }

        let repo = Repository {
            id:         Uuid::new_v4(),
            name:       name.to_owned(),
            root_path:  path.to_path_buf(),
            created_at: Utc::now(),
            mode,
        };

        let store = Store::open(path)?;
        store.insert_repo(&repo)?;

        // Create the default branch and write HEAD.
        let main = Branch {
            id:             Uuid::new_v4(),
            repo_id:        repo.id,
            name:           "main".to_owned(),
            head_commit_id: None,
            created_at:     Utc::now(),
        };
        store.insert_branch(&main)?;
        repo.write_head("main")?;

        tracing::info!(repo_id = %repo.id, path = %path.display(), "repository initialised");
        Ok(repo)
    }

    /// Opens an existing Osmos repository rooted at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let osmos_dir = path.join(".osmos");
        if !osmos_dir.exists() {
            return Err(OsmosError::RepoNotFound(path.display().to_string()));
        }
        let store = Store::open(path)?;
        store.list_repos()?
            .into_iter()
            .next()
            .ok_or_else(|| OsmosError::RepoNotFound(path.display().to_string()))
    }

    // MARK: - HEAD

    fn head_path(&self) -> PathBuf {
        self.root_path.join(".osmos").join("HEAD")
    }

    /// Returns the currently active branch name; defaults to `"main"`.
    pub fn current_branch(&self) -> String {
        std::fs::read_to_string(self.head_path())
            .map(|s| s.trim().to_owned())
            .unwrap_or_else(|_| "main".to_owned())
    }

    fn write_head(&self, branch_name: &str) -> Result<()> {
        std::fs::write(self.head_path(), branch_name)?;
        Ok(())
    }

    // MARK: - Branch operations

    /// Creates a new branch pointing at the current branch's head, optionally
    /// forking from a different `from_branch`.
    pub fn create_branch(&self, name: &str, from_branch: Option<&str>) -> Result<Branch> {
        let store = Store::open(&self.root_path)?;

        if store.get_branch(self.id, name)?.is_some() {
            return Err(OsmosError::BranchExists(name.to_owned()));
        }

        let current = self.current_branch();
        let source  = from_branch.unwrap_or(&current);
        let head_commit_id = store.get_branch(self.id, source)?
            .and_then(|b| b.head_commit_id);

        let branch = Branch {
            id:             Uuid::new_v4(),
            repo_id:        self.id,
            name:           name.to_owned(),
            head_commit_id,
            created_at:     Utc::now(),
        };
        store.insert_branch(&branch)?;
        Ok(branch)
    }

    pub fn list_branches(&self) -> Result<Vec<Branch>> {
        let store = Store::open(&self.root_path)?;
        store.list_branches(self.id)
    }

    /// Switches the working HEAD to `name`. The branch must already exist.
    pub fn switch_branch(&self, name: &str) -> Result<()> {
        let store = Store::open(&self.root_path)?;
        if store.get_branch(self.id, name)?.is_none() {
            return Err(OsmosError::BranchNotFound(name.to_owned()));
        }
        self.write_head(name)
    }

    /// Deletes a branch. Cannot delete the currently active branch.
    pub fn delete_branch(&self, name: &str) -> Result<()> {
        if self.current_branch() == name {
            return Err(OsmosError::InvalidPath(
                format!("cannot delete the currently active branch '{name}'")
            ));
        }
        let store = Store::open(&self.root_path)?;
        store.delete_branch(self.id, name)
    }

    // MARK: - Merge

    /// Merges `source_branch` into the current branch using last-write-wins
    /// for any file modified on both sides since the common ancestor.
    pub fn merge(
        &self,
        source_branch_name: &str,
        author: &str,
        message: &str,
    ) -> Result<Commit> {
        let store = Store::open(&self.root_path)?;
        let current = self.current_branch();

        let target_br = store.get_branch(self.id, &current)?
            .ok_or_else(|| OsmosError::BranchNotFound(current.clone()))?;
        let source_br = store.get_branch(self.id, source_branch_name)?
            .ok_or_else(|| OsmosError::BranchNotFound(source_branch_name.to_owned()))?;

        let target_head_id = target_br.head_commit_id
            .ok_or_else(|| OsmosError::CommitNotFound("target branch has no commits".into()))?;
        let source_head_id = source_br.head_commit_id
            .ok_or_else(|| OsmosError::CommitNotFound("source branch has no commits".into()))?;

        let target_commit = store.get_commit(target_head_id)?
            .ok_or_else(|| OsmosError::CommitNotFound(target_head_id.to_string()))?;
        let source_commit = store.get_commit(source_head_id)?
            .ok_or_else(|| OsmosError::CommitNotFound(source_head_id.to_string()))?;

        let ancestor_id = self.find_common_ancestor(&store, target_head_id, source_head_id)?;

        let target_tree   = store.get_tree_map(&target_commit.tree_hash)?;
        let source_tree   = store.get_tree_map(&source_commit.tree_hash)?;
        let ancestor_tree = if let Some(anc_id) = ancestor_id {
            let anc = store.get_commit(anc_id)?
                .ok_or_else(|| OsmosError::CommitNotFound(anc_id.to_string()))?;
            store.get_tree_map(&anc.tree_hash)?
        } else {
            HashMap::new()
        };

        // Build merged tree.
        let mut merged: HashMap<String, (String, u64)> = ancestor_tree.clone();

        let all_paths: HashSet<&String> =
            target_tree.keys().chain(source_tree.keys()).collect();

        for path in all_paths {
            let anc_hash    = ancestor_tree.get(path).map(|(h, _)| h.as_str());
            let target_hash = target_tree.get(path).map(|(h, _)| h.as_str());
            let source_hash = source_tree.get(path).map(|(h, _)| h.as_str());

            let target_changed = target_hash != anc_hash;
            let source_changed = source_hash != anc_hash;

            match (target_changed, source_changed) {
                (true, false) => {
                    // Only target changed.
                    match target_tree.get(path) {
                        Some(e) => { merged.insert(path.clone(), e.clone()); }
                        None    => { merged.remove(path); }
                    }
                }
                (false, true) => {
                    // Only source changed.
                    match source_tree.get(path) {
                        Some(e) => { merged.insert(path.clone(), e.clone()); }
                        None    => { merged.remove(path); }
                    }
                }
                (true, true) => {
                    // Both changed: last-write-wins by head commit timestamp.
                    let winner = if source_commit.created_at > target_commit.created_at {
                        source_tree.get(path)
                    } else {
                        target_tree.get(path)
                    };
                    match winner {
                        Some(e) => { merged.insert(path.clone(), e.clone()); }
                        None    => { merged.remove(path); }
                    }
                }
                (false, false) => { /* unchanged — already in `merged` */ }
            }
        }

        // Compute merged tree hash.
        let mut tree_hasher = blake3::Hasher::new();
        let mut sorted: Vec<(&String, &(String, u64))> = merged.iter().collect();
        sorted.sort_by_key(|(p, _)| *p);
        for (path, (hash, _)) in &sorted {
            tree_hasher.update(path.as_bytes());
            tree_hasher.update(hash.as_bytes());
        }
        let tree_hash = tree_hasher.finalize().to_hex().to_string();

        for (path, (blob_hash, size)) in &merged {
            store.conn.execute(
                "INSERT OR REPLACE INTO tree_entries (tree_hash, name, blob_hash, size_bytes)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![tree_hash, path, blob_hash, size],
            )?;
        }

        let merge_commit = Commit {
            id:         Uuid::new_v4(),
            repo_id:    self.id,
            parent_ids: vec![target_head_id, source_head_id],
            message:    message.to_owned(),
            tree_hash,
            author:     author.to_owned(),
            created_at: Utc::now(),
        };
        store.insert_commit(&merge_commit)?;
        store.update_branch_head(target_br.id, Some(merge_commit.id))?;

        tracing::info!(
            merge_commit_id = %merge_commit.id,
            source = source_branch_name,
            target = current,
            "merge completed"
        );
        Ok(merge_commit)
    }

    /// Finds the most recent common ancestor of two commit DAGs using BFS.
    fn find_common_ancestor(
        &self,
        store: &Store,
        a: Uuid,
        b: Uuid,
    ) -> Result<Option<Uuid>> {
        let a_set: HashSet<Uuid> = store.list_commits_from(a)?
            .into_iter()
            .map(|c| c.id)
            .collect();

        // Walk B newest-first; first match is the closest common ancestor.
        for commit in store.list_commits_from(b)? {
            if a_set.contains(&commit.id) {
                return Ok(Some(commit.id));
            }
        }
        Ok(None)
    }

    // MARK: - Working tree status

    const MAX_STATUS_FILES: usize = 500;

    pub fn status(&self) -> Result<WorkingTreeStatus> {
        let last_tree = self.last_tree_hashes()?;
        let mut status = WorkingTreeStatus::default();

        let has_commits = !last_tree.is_empty();

        for entry in walkdir::WalkDir::new(&self.root_path)
            .min_depth(1)
            .into_iter()
            .filter_entry(|e| e.file_name() != ".osmos")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let abs = entry.path();
            let rel = abs.strip_prefix(&self.root_path)
                .unwrap_or(abs)
                .to_string_lossy()
                .to_string();

            if !has_commits {
                if status.added.len() < Self::MAX_STATUS_FILES {
                    status.added.push(rel);
                }
                continue;
            }

            let current_hash = crate::hash::blake3_file(abs)?;
            match last_tree.get(&rel) {
                None if status.added.len() < Self::MAX_STATUS_FILES => {
                    status.added.push(rel);
                }
                Some(h) if h != &current_hash && status.modified.len() < Self::MAX_STATUS_FILES => {
                    status.modified.push(rel);
                }
                _ => {}
            }
        }

        for key in last_tree.keys() {
            if status.deleted.len() >= Self::MAX_STATUS_FILES { break; }
            if !self.root_path.join(key).exists() {
                status.deleted.push(key.clone());
            }
        }

        Ok(status)
    }

    /// Returns `{ relative_path → blob_hash }` for the current branch's HEAD commit tree.
    fn last_tree_hashes(&self) -> Result<HashMap<String, String>> {
        let store   = Store::open(&self.root_path)?;
        let current = self.current_branch();

        let head_id = match store.get_branch(self.id, &current)?
            .and_then(|b| b.head_commit_id)
        {
            Some(id) => id,
            None     => return Ok(HashMap::new()),
        };

        let commit = match store.get_commit(head_id)? {
            Some(c) => c,
            None    => return Ok(HashMap::new()),
        };

        let mut stmt = store.conn.prepare(
            "SELECT name, blob_hash FROM tree_entries WHERE tree_hash = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![commit.tree_hash], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut map = HashMap::new();
        for row in rows {
            let (name, hash) = row?;
            map.insert(name, hash);
        }
        Ok(map)
    }
}

/// Summary of working-tree changes relative to the last commit.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct WorkingTreeStatus {
    pub added:    Vec<String>,
    pub modified: Vec<String>,
    pub deleted:  Vec<String>,
}

impl WorkingTreeStatus {
    pub fn is_clean(&self) -> bool {
        self.added.is_empty() && self.modified.is_empty() && self.deleted.is_empty()
    }
}
