use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppErrorCode {
    InvalidInput,
    InvalidAssetRef,
    ReservedAssetKind,
    InvalidMetadata,
    InvalidAsset,
    InvalidConfig,
    ValidationFailed,
    InvalidRevision,
    UnknownRevision,
    WorkspaceNotFound,
    WorkspaceNotEmpty,
    NotGitRepository,
    WorkspaceIo,
    FileWrite,
    AssetNotFound,
    DuplicateAsset,
    GitUnavailable,
    GitCommandFailed,
    TagAlreadyExists,
    ConfirmationRequired,
    DirtyAsset,
    UnsupportedAdapter,
    DistributionConflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppError {
    pub code: AppErrorCode,
    pub operation: String,
    pub subject: String,
    pub reason: String,
    pub suggestion: String,
}

impl AppError {
    pub fn new(
        code: AppErrorCode,
        operation: impl Into<String>,
        subject: impl Into<String>,
        reason: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            code,
            operation: operation.into(),
            subject: subject.into(),
            reason: reason.into(),
            suggestion: suggestion.into(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} failed for '{}': {}. {}",
            self.operation, self.subject, self.reason, self.suggestion
        )
    }
}

impl std::error::Error for AppError {}
