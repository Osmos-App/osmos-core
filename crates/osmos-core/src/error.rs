use thiserror::Error;

#[derive(Debug, Error)]
pub enum OsmosError {
    #[error("repository not found: {0}")]
    RepoNotFound(String),

    #[error("repository already exists at: {0}")]
    RepoExists(String),

    #[error("commit not found: {0}")]
    CommitNotFound(String),

    #[error("blob not found: {hash}")]
    BlobNotFound { hash: String },

    #[error("branch not found: {0}")]
    BranchNotFound(String),

    #[error("branch already exists: {0}")]
    BranchExists(String),

    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, OsmosError>;
