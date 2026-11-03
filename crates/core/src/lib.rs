mod asset;
mod distribution;
mod error;
mod parser;
mod prompt;
mod revision;
mod validation;
mod version;
mod workspace;

pub use asset::{
    API_VERSION, Asset, AssetKind, AssetMetadata, AssetRef, AssetSnapshot, AssetSpec, PromptSpec,
    PromptVariable, SkillSpec, SnapshotFile,
};
pub use distribution::{
    AdapterFile, AdapterOutput, DistributionAdapter, DistributionConflict, DistributionExecution,
    DistributionExecutionStatus, DistributionFileResult, DistributionFileStatus, DistributionPlan,
    DistributionPlanStatus, DistributionScope, PlannedFile, PlannedFileStatus, TargetProfile,
};
pub use error::{AppError, AppErrorCode};
pub use parser::{parse_asset, parse_asset_with_sources};
pub use prompt::{PromptRenderResult, extract_prompt_variables, render_prompt};
pub use revision::{RevisionSelector, VersionTag};
pub use validation::{
    Diagnostic, DiagnosticSeverity, ValidationReport, validate_asset, validate_dependencies,
    validate_relative_asset_path,
};
pub use version::{AssetDiff, ChangeKind, FileChange, diff_snapshots};
pub use workspace::{WorkspaceDiscovery, WorkspaceInfo, WorkspaceService};
