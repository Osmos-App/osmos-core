use crate::api::{Command, Request, Response};
use osmos_core::{
    commit::Commit,
    repo::{RepoMode, Repository},
    store::Store,
};
use std::path::Path;

pub fn handle(req: Request) -> Response {
    match req.cmd {
        Command::Ping => Response::ok(req.id, serde_json::json!({ "pong": true })),

        // MARK: - Repo
        Command::InitRepo(p) => {
            let path = Path::new(&p.path);
            let mode = p.mode.parse().unwrap_or(RepoMode::Client);
            match Repository::init(path, &p.name, mode) {
                Ok(repo) => {
                    if let Ok(reg) = Store::open(&registry_path()) {
                        let _ = reg.insert_repo(&repo);
                    }
                    Response::ok(
                        req.id,
                        serde_json::json!({
                            "repo_id":    repo.id,
                            "name":       repo.name,
                            "root_path":  repo.root_path,
                            "mode":       repo.mode,
                            "created_at": repo.created_at,
                        }),
                    )
                }
                Err(osmos_core::OsmosError::RepoExists(p)) => Response::err(
                    req.id,
                    "REPO_EXISTS",
                    format!("repository already exists at {p}"),
                ),
                Err(e) => Response::err(req.id, "INTERNAL", e),
            }
        }

        Command::ListRepos => match Store::open(&registry_path()).and_then(|s| s.list_repos()) {
            Ok(repos) => Response::ok(req.id, repos),
            Err(e) => Response::err(req.id, "INTERNAL", e),
        },

        Command::GetStatus(p) => {
            match Repository::open(Path::new(&p.path)).and_then(|r| r.status()) {
                Ok(s) => Response::ok(req.id, s),
                Err(e) => Response::err(req.id, "INTERNAL", e),
            }
        }

        Command::DeleteRepo(p) => {
            if let Ok(reg) = Store::open(&registry_path()) {
                if let Err(e) = reg.delete_repo_by_path(&p.path) {
                    return Response::err(req.id, "INTERNAL", e);
                }
            }
            let osmos_dir = Path::new(&p.path).join(".osmos");
            if osmos_dir.exists() {
                if let Err(e) = std::fs::remove_dir_all(&osmos_dir) {
                    return Response::err(req.id, "INTERNAL", e.to_string());
                }
            }
            Response::ok(req.id, serde_json::json!({ "deleted": true }))
        }

        // MARK: - Commits
        Command::CreateCommit(p) => {
            let path = Path::new(&p.path);
            let repo = match Repository::open(path) {
                Ok(r) => r,
                Err(e) => return Response::err(req.id, "REPO_NOT_FOUND", e),
            };
            let store = match Store::open(path) {
                Ok(s) => s,
                Err(e) => return Response::err(req.id, "INTERNAL", e),
            };

            // Use the current branch's head as parent.
            let current_branch = repo.current_branch();
            let (branch_id, parent_ids) = match store.get_branch(repo.id, &current_branch) {
                Ok(Some(br)) => {
                    let parents = br.head_commit_id.map(|id| vec![id]).unwrap_or_default();
                    (Some(br.id), parents)
                }
                _ => (None, vec![]),
            };

            match Commit::create(path, repo.id, parent_ids, &p.message, &p.author) {
                Ok(commit) => {
                    // Advance branch head.
                    if let Some(bid) = branch_id {
                        let _ = store.update_branch_head(bid, Some(commit.id));
                    }
                    Response::ok(
                        req.id,
                        serde_json::json!({
                            "commit_id":  commit.id,
                            "tree_hash":  commit.tree_hash,
                            "created_at": commit.created_at,
                        }),
                    )
                }
                Err(e) => Response::err(req.id, "INTERNAL", e),
            }
        }

        Command::ListCommits(p) => {
            let path = Path::new(&p.path);
            let repo = match Repository::open(path) {
                Ok(r) => r,
                Err(e) => return Response::err(req.id, "REPO_NOT_FOUND", e),
            };
            let store = match Store::open(path) {
                Ok(s) => s,
                Err(e) => return Response::err(req.id, "INTERNAL", e),
            };

            // If a branch is specified, walk its DAG; otherwise return all repo commits.
            let result = if let Some(branch_name) = p.branch {
                store
                    .get_branch(repo.id, &branch_name)
                    .and_then(|opt| match opt {
                        Some(br) => match br.head_commit_id {
                            Some(head) => store.list_commits_from(head),
                            None => Ok(vec![]),
                        },
                        None => Err(osmos_core::OsmosError::BranchNotFound(branch_name)),
                    })
            } else {
                store.list_commits(repo.id)
            };

            match result {
                Ok(commits) => Response::ok(req.id, commits),
                Err(e) => Response::err(req.id, "INTERNAL", e),
            }
        }

        // MARK: - Branches
        Command::CreateBranch(p) => {
            let path = Path::new(&p.path);
            match Repository::open(path) {
                Err(e) => Response::err(req.id, "REPO_NOT_FOUND", e),
                Ok(repo) => match repo.create_branch(&p.name, p.from_branch.as_deref()) {
                    Ok(branch) => Response::ok(req.id, branch),
                    Err(osmos_core::OsmosError::BranchExists(n)) => Response::err(
                        req.id,
                        "BRANCH_EXISTS",
                        format!("branch '{n}' already exists"),
                    ),
                    Err(e) => Response::err(req.id, "INTERNAL", e),
                },
            }
        }

        Command::ListBranches(p) => {
            let path = Path::new(&p.path);
            match Repository::open(path) {
                Err(e) => Response::err(req.id, "REPO_NOT_FOUND", e),
                Ok(repo) => match repo.list_branches() {
                    Ok(branches) => Response::ok(req.id, branches),
                    Err(e) => Response::err(req.id, "INTERNAL", e),
                },
            }
        }

        Command::SwitchBranch(p) => {
            let path = Path::new(&p.path);
            match Repository::open(path) {
                Err(e) => Response::err(req.id, "REPO_NOT_FOUND", e),
                Ok(repo) => match repo.switch_branch(&p.name) {
                    Ok(()) => Response::ok(req.id, serde_json::json!({ "current_branch": p.name })),
                    Err(osmos_core::OsmosError::BranchNotFound(n)) => Response::err(
                        req.id,
                        "BRANCH_NOT_FOUND",
                        format!("branch '{n}' not found"),
                    ),
                    Err(e) => Response::err(req.id, "INTERNAL", e),
                },
            }
        }

        Command::MergeBranch(p) => {
            let path = Path::new(&p.path);
            match Repository::open(path) {
                Err(e) => Response::err(req.id, "REPO_NOT_FOUND", e),
                Ok(repo) => match repo.merge(&p.source_branch, &p.author, &p.message) {
                    Ok(commit) => Response::ok(
                        req.id,
                        serde_json::json!({
                            "commit_id":  commit.id,
                            "tree_hash":  commit.tree_hash,
                            "created_at": commit.created_at,
                        }),
                    ),
                    Err(osmos_core::OsmosError::BranchNotFound(n)) => Response::err(
                        req.id,
                        "BRANCH_NOT_FOUND",
                        format!("branch '{n}' not found"),
                    ),
                    Err(e) => Response::err(req.id, "INTERNAL", e),
                },
            }
        }

        Command::DeleteBranch(p) => {
            let path = Path::new(&p.path);
            match Repository::open(path) {
                Err(e) => Response::err(req.id, "REPO_NOT_FOUND", e),
                Ok(repo) => match repo.delete_branch(&p.name) {
                    Ok(()) => Response::ok(req.id, serde_json::json!({ "deleted": true })),
                    Err(e) => Response::err(req.id, "INTERNAL", e),
                },
            }
        }

        Command::GetCurrentBranch(p) => {
            let path = Path::new(&p.path);
            match Repository::open(path) {
                Err(e) => Response::err(req.id, "REPO_NOT_FOUND", e),
                Ok(repo) => Response::ok(
                    req.id,
                    serde_json::json!({
                        "current_branch": repo.current_branch()
                    }),
                ),
            }
        }
    }
}

fn registry_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".osmos")
}
