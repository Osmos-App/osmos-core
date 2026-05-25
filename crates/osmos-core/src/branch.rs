use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// A named pointer to a commit — the branch head advances with each new commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub id:             Uuid,
    pub repo_id:        Uuid,
    pub name:           String,
    /// `None` for a freshly created branch with no commits yet.
    pub head_commit_id: Option<Uuid>,
    pub created_at:     DateTime<Utc>,
}
