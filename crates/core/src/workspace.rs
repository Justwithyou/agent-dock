use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::{AppError, AppErrorCode};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub root: PathBuf,
    pub config_path: PathBuf,
    pub git_root: Option<PathBuf>,
}

pub trait WorkspaceDiscovery: Send + Sync {
    fn discover(&self, start: &Path) -> Result<WorkspaceInfo, AppError>;
}

#[derive(Clone)]
pub struct WorkspaceService {
    discovery: Arc<dyn WorkspaceDiscovery>,
}

impl WorkspaceService {
    pub fn new<D>(discovery: D) -> Self
    where
        D: WorkspaceDiscovery + 'static,
    {
        Self {
            discovery: Arc::new(discovery),
        }
    }

    pub fn discover(&self, start: Option<&Path>) -> Result<WorkspaceInfo, AppError> {
        let start = match start {
            Some(path) => path.to_path_buf(),
            None => std::env::current_dir().map_err(|error| {
                AppError::new(
                    AppErrorCode::WorkspaceIo,
                    "workspace.current_dir",
                    "current directory",
                    error.to_string(),
                    "Pass an explicit workspace path",
                )
            })?,
        };
        self.discovery.discover(&start)
    }
}
