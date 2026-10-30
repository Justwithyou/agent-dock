use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{AppError, AppErrorCode, AssetRef, AssetSnapshot, RevisionSelector};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetProfile {
    pub target: String,
    pub root: PathBuf,
}

impl TargetProfile {
    pub fn new(target: impl Into<String>, root: PathBuf) -> Result<Self, AppError> {
        let target = target.into();
        if target.trim().is_empty() || target.trim() != target {
            return Err(AppError::new(
                AppErrorCode::InvalidInput,
                "target_profile.validate",
                target,
                "target must be a non-empty identifier without surrounding whitespace",
                "Use a configured target identifier such as claude-code",
            ));
        }
        if root.as_os_str().is_empty() {
            return Err(AppError::new(
                AppErrorCode::InvalidConfig,
                "target_profile.validate",
                target,
                "target root cannot be empty",
                "Configure an absolute or workspace-relative target root",
            ));
        }
        Ok(Self { target, root })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterFile {
    pub path: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterOutput {
    pub adapter_version: String,
    pub files: Vec<AdapterFile>,
}

pub trait DistributionAdapter: Send + Sync {
    fn adapt(&self, snapshot: &AssetSnapshot) -> Result<AdapterOutput, AppError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionPlanStatus {
    Ready,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannedFileStatus {
    Added,
    Updated,
    Unchanged,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionConflict {
    UnmanagedFile,
    LocallyModified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedFile {
    pub target_path: PathBuf,
    pub relative_path: String,
    pub checksum: String,
    pub content: Vec<u8>,
    pub status: PlannedFileStatus,
    pub conflict: Option<DistributionConflict>,
    pub previous_checksum: Option<String>,
    pub managed_checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionPlan {
    pub asset: AssetRef,
    pub target: String,
    pub target_root: PathBuf,
    pub source: RevisionSelector,
    pub commit: String,
    pub adapter_version: String,
    pub status: DistributionPlanStatus,
    pub files: Vec<PlannedFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionExecutionStatus {
    Completed,
    PartialFailed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionFileStatus {
    Written,
    Unchanged,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionFileResult {
    pub target_path: PathBuf,
    pub relative_path: String,
    pub status: DistributionFileStatus,
    pub error: Option<AppError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionExecution {
    pub status: DistributionExecutionStatus,
    pub files: Vec<DistributionFileResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_profile_rejects_empty_names_and_roots() {
        assert!(TargetProfile::new(" ", PathBuf::from("target")).is_err());
        assert!(TargetProfile::new("cursor", PathBuf::new()).is_err());
    }

    #[test]
    fn distribution_models_have_stable_snake_case_status_serialization() {
        let plan = DistributionPlan {
            asset: AssetRef::parse("prompt:review").unwrap(),
            target: "cursor".to_owned(),
            target_root: PathBuf::from("target"),
            source: RevisionSelector::Head,
            commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            adapter_version: "cursor-prompt/v1".to_owned(),
            status: DistributionPlanStatus::Blocked,
            files: vec![PlannedFile {
                target_path: PathBuf::from("target/skills/review/SKILL.md"),
                relative_path: "skills/review/SKILL.md".to_owned(),
                checksum: "abc".to_owned(),
                content: b"content".to_vec(),
                status: PlannedFileStatus::Conflict,
                conflict: Some(DistributionConflict::UnmanagedFile),
                previous_checksum: Some("old".to_owned()),
                managed_checksum: None,
            }],
        };

        let json = serde_json::to_string(&plan).unwrap();
        assert!(json.contains("\"status\":\"blocked\""));
        assert!(json.contains("\"conflict\":\"unmanaged_file\""));
        assert_eq!(
            serde_json::from_str::<DistributionPlan>(&json).unwrap(),
            plan
        );
    }
}
