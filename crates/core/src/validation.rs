use std::{
    collections::{BTreeSet, HashSet},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{AppError, AppErrorCode, Asset, AssetRef, AssetSpec, PromptSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: AppErrorCode,
    pub asset: Option<AssetRef>,
    pub path: PathBuf,
    pub field: Option<String>,
    pub message: String,
}

impl Diagnostic {
    pub fn error(
        code: AppErrorCode,
        asset: Option<AssetRef>,
        path: PathBuf,
        field: Option<&str>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: DiagnosticSeverity::Error,
            code,
            asset,
            path,
            field: field.map(str::to_owned),
            message: message.into(),
        }
    }

    pub fn warning(
        code: AppErrorCode,
        asset: Option<AssetRef>,
        path: PathBuf,
        field: Option<&str>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: DiagnosticSeverity::Warning,
            code,
            asset,
            path,
            field: field.map(str::to_owned),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub asset_count: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
}

impl ValidationReport {
    pub fn new(asset_count: usize, mut diagnostics: Vec<Diagnostic>) -> Self {
        diagnostics.sort_by(|left, right| {
            (&left.path, &left.field, &left.message).cmp(&(
                &right.path,
                &right.field,
                &right.message,
            ))
        });
        let error_count = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .count();
        let warning_count = diagnostics.len() - error_count;
        Self {
            asset_count,
            error_count,
            warning_count,
            valid: error_count == 0,
            diagnostics,
        }
    }
}

pub fn validate_asset(asset: &Asset) -> Vec<Diagnostic> {
    let reference = asset.reference().ok();
    let path = asset
        .source_files
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("<unknown>"));
    let mut diagnostics = Vec::new();
    if let Err(error) = asset.metadata.validate() {
        diagnostics.push(Diagnostic::error(
            error.code,
            reference.clone(),
            path.clone(),
            Some("metadata"),
            error.reason,
        ));
    }
    if asset.body.trim().is_empty() {
        diagnostics.push(Diagnostic::error(
            AppErrorCode::InvalidAsset,
            reference.clone(),
            path.clone(),
            Some("body"),
            "Markdown body is required",
        ));
    }
    add_duplicate_diagnostics(
        &asset.metadata.owners,
        "owners",
        "owner",
        &reference,
        &path,
        &mut diagnostics,
    );
    add_duplicate_diagnostics(
        &asset.metadata.labels,
        "labels",
        "classification label",
        &reference,
        &path,
        &mut diagnostics,
    );

    match &asset.spec {
        AssetSpec::Prompt(spec) => validate_prompt_spec(spec, &reference, &path, &mut diagnostics),
        AssetSpec::Skill(spec) => {
            let skill_root = reference.as_ref().map(AssetRef::root_path);
            for (index, source) in asset.source_files.iter().enumerate() {
                let field = format!("source_files[{index}]");
                if let Err(error) = validate_relative_asset_path(source) {
                    diagnostics.push(Diagnostic::error(
                        AppErrorCode::InvalidAsset,
                        reference.clone(),
                        source.clone(),
                        Some(&field),
                        error.reason,
                    ));
                } else if skill_root
                    .as_ref()
                    .is_some_and(|root| !source.starts_with(root))
                {
                    diagnostics.push(Diagnostic::error(
                        AppErrorCode::InvalidAsset,
                        reference.clone(),
                        source.clone(),
                        Some(&field),
                        "Skill source files must stay inside the Skill directory",
                    ));
                }
            }
            for (index, trigger) in spec.triggers.iter().enumerate() {
                if trigger.trim().is_empty() {
                    diagnostics.push(Diagnostic::error(
                        AppErrorCode::InvalidAsset,
                        reference.clone(),
                        path.clone(),
                        Some(&format!("spec.triggers[{index}]")),
                        "trigger cannot be empty",
                    ));
                }
            }
            if let Some(entrypoint) = &spec.entrypoint {
                if let Err(error) = validate_relative_asset_path(entrypoint) {
                    diagnostics.push(Diagnostic::error(
                        AppErrorCode::InvalidAsset,
                        reference.clone(),
                        path.clone(),
                        Some("spec.entrypoint"),
                        error.reason,
                    ));
                } else if let Some(entry) = asset.source_files.first() {
                    let expected = entry
                        .parent()
                        .unwrap_or_else(|| Path::new(""))
                        .join(entrypoint);
                    if !asset.source_files.contains(&expected) {
                        diagnostics.push(Diagnostic::error(
                            AppErrorCode::AssetNotFound,
                            reference.clone(),
                            expected,
                            Some("spec.entrypoint"),
                            "entrypoint does not exist in the Skill source files",
                        ));
                    }
                }
            }
        }
    }
    diagnostics
}

pub fn validate_dependencies(asset: &Asset, available: &BTreeSet<AssetRef>) -> Vec<Diagnostic> {
    let AssetSpec::Skill(spec) = &asset.spec else {
        return Vec::new();
    };
    let reference = asset.reference().ok();
    let path = asset
        .source_files
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("<unknown>"));
    let mut diagnostics = Vec::new();
    let mut seen = HashSet::new();
    for (index, dependency) in spec.dependencies.iter().enumerate() {
        let field = format!("spec.dependencies[{index}]");
        if !seen.insert(dependency) {
            diagnostics.push(Diagnostic::error(
                AppErrorCode::InvalidAsset,
                reference.clone(),
                path.clone(),
                Some(&field),
                "duplicate dependency declaration",
            ));
            continue;
        }
        if let Some(tool) = dependency.strip_prefix("tool:") {
            if tool.trim().is_empty() || tool.trim() != tool {
                diagnostics.push(Diagnostic::error(
                    AppErrorCode::InvalidAsset,
                    reference.clone(),
                    path.clone(),
                    Some(&field),
                    "external tool dependency must use tool:<name>",
                ));
            }
            continue;
        }
        match AssetRef::parse(dependency) {
            Ok(local) if local.kind != asset.metadata.kind || local.id != asset.metadata.id => {
                if !available.contains(&local) {
                    diagnostics.push(Diagnostic::warning(
                        AppErrorCode::AssetNotFound,
                        reference.clone(),
                        path.clone(),
                        Some(&field),
                        format!("local dependency {local} is missing; it will not be installed automatically"),
                    ));
                }
            }
            Ok(_) => diagnostics.push(Diagnostic::error(
                AppErrorCode::InvalidAsset,
                reference.clone(),
                path.clone(),
                Some(&field),
                "an asset cannot depend on itself",
            )),
            Err(error) => diagnostics.push(Diagnostic::error(
                error.code,
                reference.clone(),
                path.clone(),
                Some(&field),
                format!("invalid dependency '{dependency}': {}", error.reason),
            )),
        }
    }
    diagnostics
}

pub fn validate_relative_asset_path(path: &Path) -> Result<(), AppError> {
    let display = path.to_string_lossy();
    let valid = !display.is_empty()
        && !path.is_absolute()
        && !display.contains('\\')
        && !display.contains(':')
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
    if valid
        && path
            .components()
            .any(|component| matches!(component, Component::Normal(_)))
    {
        return Ok(());
    }
    Err(AppError::new(
        AppErrorCode::InvalidAsset,
        "skill.path.validate",
        display,
        "Skill paths must be portable relative paths inside the Skill directory",
        "Use a forward-slash relative path without .., drive letters, or absolute roots",
    ))
}

fn validate_prompt_spec(
    spec: &PromptSpec,
    reference: &Option<AssetRef>,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = HashSet::new();
    for (index, variable) in spec.variables.iter().enumerate() {
        let field = format!("spec.variables[{index}]");
        if !valid_variable_name(&variable.name) {
            diagnostics.push(Diagnostic::error(
                AppErrorCode::InvalidAsset,
                reference.clone(),
                path.to_path_buf(),
                Some(&format!("{field}.name")),
                "variable names must match [A-Za-z_][A-Za-z0-9_]*",
            ));
        }
        if !seen.insert(&variable.name) {
            diagnostics.push(Diagnostic::error(
                AppErrorCode::InvalidAsset,
                reference.clone(),
                path.to_path_buf(),
                Some(&format!("{field}.name")),
                format!("duplicate variable declaration '{}'", variable.name),
            ));
        }
    }
}

pub(crate) fn valid_variable_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn add_duplicate_diagnostics(
    values: &[String],
    field: &str,
    description: &str,
    reference: &Option<AssetRef>,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value) {
            diagnostics.push(Diagnostic::error(
                AppErrorCode::InvalidMetadata,
                reference.clone(),
                path.to_path_buf(),
                Some(field),
                format!("duplicate {description} '{value}'"),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{API_VERSION, AssetKind, AssetMetadata, SkillSpec};

    use super::*;

    #[test]
    fn relative_skill_paths_accept_portable_and_reject_absolute_or_escaping_values() {
        for path in ["scripts/run.sh", "./scripts/run.sh", "references/guide.md"] {
            assert!(
                validate_relative_asset_path(Path::new(path)).is_ok(),
                "{path}"
            );
        }
        for path in [
            "",
            ".",
            "/tmp/run.sh",
            "C:/tmp/run.sh",
            "//server/share/run.sh",
            "scripts\\run.sh",
            "../outside.sh",
            "scripts/../outside.sh",
        ] {
            assert!(
                validate_relative_asset_path(Path::new(path)).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn skill_entrypoint_must_exist_and_valid_multi_file_skill_has_no_diagnostics() {
        let valid = skill_asset(
            "scripts/run.sh",
            &[
                "skills/onboard/SKILL.md",
                "skills/onboard/scripts/run.sh",
                "skills/onboard/references/guide.md",
            ],
        );
        assert!(validate_asset(&valid).is_empty());

        let missing = skill_asset(
            "scripts/missing.sh",
            &[
                "skills/onboard/SKILL.md",
                "skills/onboard/references/guide.md",
            ],
        );
        let diagnostics = validate_asset(&missing);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == AppErrorCode::AssetNotFound
                && diagnostic.field.as_deref() == Some("spec.entrypoint")
        }));
    }

    #[test]
    fn skill_resources_must_stay_inside_skill_root() {
        for invalid in [
            "/tmp/guide.md",
            "C:/tmp/guide.md",
            "skills\\onboard\\guide.md",
            "skills/onboard/../outside.md",
            "skills/other/guide.md",
            "prompts/review.md",
        ] {
            let asset = skill_asset(
                "scripts/run.sh",
                &[
                    "skills/onboard/SKILL.md",
                    "skills/onboard/scripts/run.sh",
                    invalid,
                ],
            );
            let diagnostics = validate_asset(&asset);
            assert!(
                diagnostics.iter().any(|diagnostic| {
                    diagnostic.code == AppErrorCode::InvalidAsset
                        && diagnostic.field.as_deref() == Some("source_files[2]")
                }),
                "{invalid}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn skill_entrypoint_cannot_be_relocated_to_another_skill_root() {
        let asset = skill_asset(
            "scripts/run.sh",
            &["skills/other/SKILL.md", "skills/other/scripts/run.sh"],
        );
        let diagnostics = validate_asset(&asset);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == AppErrorCode::InvalidAsset
                && diagnostic.field.as_deref() == Some("source_files[0]")
        }));
    }

    fn skill_asset(entrypoint: &str, source_files: &[&str]) -> Asset {
        Asset {
            metadata: AssetMetadata {
                api_version: API_VERSION.to_owned(),
                kind: AssetKind::Skill,
                id: "onboard".to_owned(),
                title: "Onboard".to_owned(),
                description: "Onboard a repository".to_owned(),
                owners: vec!["team".to_owned()],
                labels: Vec::new(),
            },
            spec: AssetSpec::Skill(SkillSpec {
                triggers: Vec::new(),
                dependencies: Vec::new(),
                entrypoint: Some(PathBuf::from(entrypoint)),
            }),
            body: "Inspect the repository.".to_owned(),
            source_files: source_files.iter().map(PathBuf::from).collect(),
        }
    }
}
