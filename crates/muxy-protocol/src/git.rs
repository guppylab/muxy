use serde::{Deserialize, Serialize};

use crate::{ErrorCode, OperationId, ProjectDescriptor, ProjectId, ServerPath};

/// Git operations always resolve their repository through a server project.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitRequest {
    pub project: ProjectId,
    pub action: GitAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum GitAction {
    Summary,
    Branches,
    Changes,
    SwitchBranch(String),
    CreateBranch(String),
    DeleteBranch(String),
    Stage(Vec<ServerPath>),
    Unstage(Vec<ServerPath>),
    Discard(Vec<ServerPath>),
    Worktrees,
    InspectRemoval,
    Worktree(WorktreeIntent),
    /// Replace this connection's filesystem watch with the requested project.
    Watch,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorktreeIntent {
    pub operation: OperationId,
    pub action: WorktreeAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum WorktreeAction {
    Create {
        project: ProjectId,
        directory: ServerPath,
        branch: String,
        /// None checks out an existing branch; Some creates a branch from this ref.
        base: Option<String>,
    },
    Register {
        project: ProjectId,
        directory: ServerPath,
    },
    Remove {
        expected: WorktreeRemoval,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitSummary {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u64,
    pub behind: u64,
    pub changed: u32,
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub conflicted: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitBranch {
    pub name: String,
    pub current: bool,
    pub checked_out: bool,
    pub default: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitFile {
    pub path: ServerPath,
    pub original_path: Option<ServerPath>,
    pub index: u8,
    pub worktree: u8,
    pub added: Option<u64>,
    pub removed: Option<u64>,
}

impl GitFile {
    pub fn untracked(&self) -> bool {
        self.index == b'?'
    }
    pub fn conflicted(&self) -> bool {
        self.index == b'U'
            || self.worktree == b'U'
            || matches!((self.index, self.worktree), (b'A', b'A') | (b'D', b'D'))
    }
    pub fn staged(&self) -> bool {
        !self.untracked() && self.index != b' '
    }
    pub fn unstaged(&self) -> bool {
        !self.untracked() && self.worktree != b' '
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitWorktree {
    pub directory: ServerPath,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub primary: bool,
    pub locked: bool,
    pub registered: Option<ProjectId>,
}

/// An inspection binds confirmation to the directory identity and current Git state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorktreeRemoval {
    pub directory: ServerPath,
    pub device: u64,
    pub inode: u64,
    pub dirty: bool,
    pub status: Vec<u8>,
    pub head: Option<String>,
    pub branch: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum GitReply {
    /// None means an existing directory without a Git repository.
    Summary(Option<GitSummary>),
    Branches(Vec<GitBranch>),
    Changes(Vec<GitFile>),
    Worktrees(Vec<GitWorktree>),
    Removal(WorktreeRemoval),
    Project(ProjectDescriptor),
    Done,
}

impl GitRequest {
    pub fn validate(&self) -> Result<(), ErrorCode> {
        fn text(value: &str) -> Result<(), ErrorCode> {
            if value.is_empty()
                || value.len() > 1024
                || value.contains('\0')
                || value.starts_with('-')
            {
                Err(ErrorCode::BadRequest)
            } else {
                Ok(())
            }
        }
        fn path(value: &ServerPath) -> Result<(), ErrorCode> {
            if value.0.is_empty() || value.0.len() > 4096 || value.0.contains(&0) {
                Err(ErrorCode::BadPath)
            } else {
                Ok(())
            }
        }
        match &self.action {
            GitAction::SwitchBranch(s)
            | GitAction::CreateBranch(s)
            | GitAction::DeleteBranch(s) => text(s),
            GitAction::Stage(paths) | GitAction::Unstage(paths) | GitAction::Discard(paths) => {
                if paths.is_empty() || paths.len() > 4096 {
                    return Err(ErrorCode::BadRequest);
                }
                paths.iter().try_for_each(path)
            }
            GitAction::Worktree(intent) => match &intent.action {
                WorktreeAction::Create {
                    directory,
                    branch,
                    base,
                    ..
                } => {
                    path(directory)?;
                    text(branch)?;
                    base.as_deref().map_or(Ok(()), text)
                }
                WorktreeAction::Register { directory, .. } => path(directory),
                WorktreeAction::Remove { expected } => {
                    if expected.status.len() > 4 * 1024 * 1024 {
                        return Err(ErrorCode::BadRequest);
                    }
                    path(&expected.directory)
                }
            },
            _ => Ok(()),
        }
    }
}
