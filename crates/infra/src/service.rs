use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use adock_core::{
    AppError, AppErrorCode, Asset, AssetKind, AssetRef, Diagnostic, PromptRenderResult,
    ValidationReport, render_prompt, validate_asset as diagnostics_for_asset,
    validate_dependencies,
};
use serde::{Deserialize, Serialize};

use crate::{ScanIssue, WorkspaceScanner};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetFilter {
    pub kind: Option<AssetKind>,
    pub owner: Option<String>,
    pub label: Option<String>,
    pub query: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetListItem {
    pub asset: AssetRef,
    pub title: String,
    pub description: String,
    pub owners: Vec<String>,
    pub labels: Vec<String>,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetListResult {
    pub assets: Vec<AssetListItem>,
    pub issues: Vec<ScanIssue>,
}

#[derive(Debug, Clone)]
pub struct AssetService {
    root: PathBuf,
}

impl AssetService {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn list(&self, filter: &AssetFilter) -> Result<AssetListResult, AppError> {
        let scan = WorkspaceScanner::new(&self.root).scan()?;
        let query = filter.query.as_ref().map(|query| query.to_lowercase());
        let assets = scan
            .assets
            .into_iter()
            .filter(|asset| filter.kind.is_none_or(|kind| kind == asset.metadata.kind))
            .filter(|asset| {
                filter.owner.as_ref().is_none_or(|owner| {
                    asset
                        .metadata
                        .owners
                        .iter()
                        .any(|candidate| candidate == owner)
                })
            })
            .filter(|asset| {
                filter.label.as_ref().is_none_or(|label| {
                    asset
                        .metadata
                        .labels
                        .iter()
                        .any(|candidate| candidate == label)
                })
            })
            .filter(|asset| {
                query.as_ref().is_none_or(|query| {
                    asset.metadata.id.to_lowercase().contains(query)
                        || asset.metadata.title.to_lowercase().contains(query)
                        || asset.metadata.description.to_lowercase().contains(query)
                })
            })
            .map(list_item)
            .collect();
        Ok(AssetListResult {
            assets,
            issues: scan.issues,
        })
    }

    pub fn show(&self, reference: &AssetRef) -> Result<Asset, AppError> {
        reference.ensure_supported()?;
        let scan = WorkspaceScanner::new(&self.root).scan()?;
        if let Some(issue) = scan.issues.iter().find(|issue| {
            issue.asset.as_ref() == Some(reference) || issue.path == reference.entry_path()
        }) {
            return Err(issue.error.clone());
        }
        scan.assets
            .into_iter()
            .find(|asset| asset.reference().ok().as_ref() == Some(reference))
            .ok_or_else(|| not_found(reference))
    }

    pub fn validate(&self, reference: Option<&AssetRef>) -> Result<ValidationReport, AppError> {
        if let Some(reference) = reference {
            reference.ensure_supported()?;
        }
        let scan = WorkspaceScanner::new(&self.root).scan()?;
        let available = scan
            .assets
            .iter()
            .filter_map(|asset| asset.reference().ok())
            .collect::<BTreeSet<_>>();
        let mut diagnostics = scan.issues.iter().map(issue_diagnostic).collect::<Vec<_>>();
        for asset in &scan.assets {
            diagnostics.extend(diagnostics_for_asset(asset));
            diagnostics.extend(validate_dependencies(asset, &available));
        }

        if let Some(reference) = reference {
            let entry_path = reference.entry_path();
            let found = scan
                .assets
                .iter()
                .any(|asset| asset.reference().ok().as_ref() == Some(reference))
                || scan.issues.iter().any(|issue| {
                    issue.asset.as_ref() == Some(reference) || issue.path == entry_path
                });
            if !found {
                return Err(not_found(reference));
            }
            diagnostics.retain(|diagnostic| {
                diagnostic.asset.as_ref() == Some(reference)
                    || diagnostic.path == entry_path
                    || (reference.kind == AssetKind::Skill
                        && diagnostic.path.starts_with(reference.root_path()))
            });
            let asset_count = scan
                .assets
                .iter()
                .filter(|asset| asset.reference().ok().as_ref() == Some(reference))
                .count();
            return Ok(ValidationReport::new(asset_count, diagnostics));
        }
        Ok(ValidationReport::new(scan.assets.len(), diagnostics))
    }

    pub fn render_prompt(
        &self,
        reference: &AssetRef,
        values: Option<&BTreeMap<String, String>>,
    ) -> Result<PromptRenderResult, AppError> {
        let asset = self.show(reference)?;
        render_prompt(&asset, values)
    }
}

fn list_item(asset: Asset) -> AssetListItem {
    AssetListItem {
        asset: asset
            .reference()
            .expect("scanner only returns valid references"),
        title: asset.metadata.title,
        description: asset.metadata.description,
        owners: asset.metadata.owners,
        labels: asset.metadata.labels,
        source_path: asset.source_files.first().cloned().unwrap_or_default(),
    }
}

fn issue_diagnostic(issue: &ScanIssue) -> Diagnostic {
    Diagnostic::error(
        issue.error.code,
        issue.asset.clone(),
        issue.path.clone(),
        None,
        issue.error.reason.clone(),
    )
}

fn not_found(reference: &AssetRef) -> AppError {
    AppError::new(
        AppErrorCode::AssetNotFound,
        "assets.lookup",
        reference.to_string(),
        "asset was not found in the workspace",
        "Run list to see available assets and check the kind and id",
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use adock_core::{AssetSpec, DiagnosticSeverity};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn list_and_validate_keep_valid_assets_available_when_another_is_broken() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("prompts")).unwrap();
        fs::write(temp.path().join("prompts/bad.md"), "broken").unwrap();
        fs::write(
            temp.path().join("prompts/review.md"),
            "---\napiVersion: adock/v1alpha1\nkind: prompt\nid: review\ntitle: Review\ndescription: Review code\nowners: [alice, alice]\nspec:\n  variables:\n    - name: path\n      required: true\n    - name: path\n---\nReview {{path}}\n",
        )
        .unwrap();
        let service = AssetService::new(temp.path());
        let list = service.list(&AssetFilter::default()).unwrap();
        assert_eq!(list.assets.len(), 1);
        assert_eq!(list.issues.len(), 1);
        let report = service.validate(None).unwrap();
        assert!(!report.valid);
        assert!(report.error_count >= 3);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate variable"))
        );
    }

    #[test]
    fn missing_dependencies_are_warnings_without_recursive_installation() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("skills/demo")).unwrap();
        fs::write(
            temp.path().join("skills/demo/SKILL.md"),
            "---\napiVersion: adock/v1alpha1\nkind: skill\nid: demo\ntitle: Demo\ndescription: Demo skill\nowners: [bob]\nspec:\n  dependencies: [prompt:missing, tool:rg]\n---\nInstructions\n",
        )
        .unwrap();
        let service = AssetService::new(temp.path());
        let report = service.validate(None).unwrap();
        assert!(report.valid);
        assert_eq!(report.warning_count, 1);
        assert_eq!(report.diagnostics[0].severity, DiagnosticSeverity::Warning);
        let asset = service
            .show(&AssetRef::parse("skill:demo").unwrap())
            .unwrap();
        assert!(matches!(asset.spec, AssetSpec::Skill(_)));
    }
}
