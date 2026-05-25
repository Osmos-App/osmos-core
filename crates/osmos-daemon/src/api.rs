use serde::{Deserialize, Serialize};
use uuid::Uuid;

// MARK: - Wire types

#[derive(Debug, Deserialize)]
pub struct Request {
    pub id:  Uuid,
    pub cmd: Command,
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub id:   Uuid,
    pub ok:   bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: String,
    pub msg:  String,
}

impl Response {
    pub fn ok(id: Uuid, data: impl Serialize) -> Self {
        Self {
            id,
            ok:    true,
            data:  Some(serde_json::to_value(data).unwrap_or(serde_json::Value::Null)),
            error: None,
        }
    }

    pub fn err(id: Uuid, code: &str, msg: impl std::fmt::Display) -> Self {
        Self {
            id,
            ok:    false,
            data:  None,
            error: Some(ApiError { code: code.to_owned(), msg: msg.to_string() }),
        }
    }
}

// MARK: - Commands

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    // Repo management
    InitRepo(InitRepoParams),
    ListRepos,
    GetStatus(RepoPathParams),
    DeleteRepo(RepoPathParams),

    // Commits
    CreateCommit(CreateCommitParams),
    ListCommits(ListCommitsParams),

    // Branches
    CreateBranch(CreateBranchParams),
    ListBranches(RepoPathParams),
    SwitchBranch(SwitchBranchParams),
    MergeBranch(MergeBranchParams),
    DeleteBranch(DeleteBranchParams),
    GetCurrentBranch(RepoPathParams),

    /// Ping — daemon replies with `{"pong": true}`.
    Ping,
}

// MARK: - Param structs

#[derive(Debug, Deserialize)]
pub struct InitRepoParams {
    pub path: String,
    pub name: String,
    #[serde(default = "default_client")]
    pub mode: String,
}

fn default_client() -> String { "client".to_owned() }

#[derive(Debug, Deserialize)]
pub struct RepoPathParams {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateCommitParams {
    pub path:    String,
    pub message: String,
    pub author:  String,
}

#[derive(Debug, Deserialize)]
pub struct ListCommitsParams {
    pub path: String,
    /// If provided, walk DAG from this branch's head; otherwise returns all repo commits.
    pub branch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBranchParams {
    pub path:        String,
    pub name:        String,
    /// Fork from this branch instead of the current HEAD.
    pub from_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SwitchBranchParams {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct MergeBranchParams {
    pub path:          String,
    pub source_branch: String,
    pub message:       String,
    pub author:        String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteBranchParams {
    pub path: String,
    pub name: String,
}
