use std::{fmt, path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{AppError, AppErrorCode, RevisionSelector};

pub const API_VERSION: &str = "adock/v1alpha1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Prompt,
    Skill,
    #[serde(rename = "mcp")]
    McpReserved,
}

impl AssetKind {
    pub fn ensure_supported(self) -> Result<(), AppError> {
        if self == Self::McpReserved {
            return Err(AppError::new(
                AppErrorCode::ReservedAssetKind,
                "asset.ensure_supported",
                self.to_string(),
                "MCP is a reserved asset kind and is outside the current MVP",
                "Use prompt or skill instead",
            ));
        }
        Ok(())
    }
}

impl fmt::Display for AssetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Prompt => "prompt",
            Self::Skill => "skill",
            Self::McpReserved => "mcp",
        })
    }
}

impl FromStr for AssetKind {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "prompt" => Ok(Self::Prompt),
            "skill" => Ok(Self::Skill),
            "mcp" => Ok(Self::McpReserved),
            _ => Err(AppError::new(
                AppErrorCode::InvalidAssetRef,
                "asset_kind.parse",
                value,
                "unknown asset kind",
                "Use prompt, skill, or the reserved mcp kind",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AssetRef {
    pub kind: AssetKind,
    pub id: String,
}

impl AssetRef {
    pub fn new(kind: AssetKind, id: impl Into<String>) -> Result<Self, AppError> {
        let id = id.into();
        validate_asset_id(&id)?;
        Ok(Self { kind, id })
    }

    pub fn parse(value: &str) -> Result<Self, AppError> {
        value.parse()
    }

    pub fn ensure_supported(&self) -> Result<(), AppError> {
        self.kind.ensure_supported()
    }

    pub fn entry_path(&self) -> PathBuf {
        match self.kind {
            AssetKind::Prompt => PathBuf::from("prompts").join(format!("{}.md", self.id)),
            AssetKind::Skill => PathBuf::from("skills").join(&self.id).join("SKILL.md"),
            AssetKind::McpReserved => PathBuf::from("mcp").join(&self.id),
        }
    }

    pub fn root_path(&self) -> PathBuf {
        match self.kind {
            AssetKind::Skill => PathBuf::from("skills").join(&self.id),
            _ => self.entry_path(),
        }
    }
}

impl fmt::Display for AssetRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.kind, self.id)
    }
}

impl FromStr for AssetRef {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (kind, id) = value.split_once(':').ok_or_else(|| {
            AppError::new(
                AppErrorCode::InvalidAssetRef,
                "asset_ref.parse",
                value,
                "expected <kind>:<id>",
                "Use a reference such as prompt:code-review",
            )
        })?;
        Self::new(kind.parse()?, id)
    }
}

fn validate_asset_id(id: &str) -> Result<(), AppError> {
    let valid = !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && id.bytes().any(|byte| byte.is_ascii_alphanumeric())
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--");

    if valid {
        Ok(())
    } else {
        Err(AppError::new(
            AppErrorCode::InvalidAssetRef,
            "asset_id.validate",
            id,
            "asset IDs must contain lowercase letters, digits, or single hyphens and start and end with an alphanumeric character",
            "Use an ID such as code-review",
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetMetadata {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: AssetKind,
    pub id: String,
    pub title: String,
    pub description: String,
    pub owners: Vec<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

impl AssetMetadata {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.api_version != API_VERSION {
            return Err(metadata_error(
                &self.id,
                "apiVersion",
                format!("expected {API_VERSION}"),
            ));
        }
        validate_asset_id(&self.id)?;
        if self.title.trim().is_empty() {
            return Err(metadata_error(&self.id, "title", "title is required"));
        }
        if self.description.trim().is_empty() {
            return Err(metadata_error(
                &self.id,
                "description",
                "description is required",
            ));
        }
        if self.owners.is_empty() || self.owners.iter().any(|owner| owner.trim().is_empty()) {
            return Err(metadata_error(
                &self.id,
                "owners",
                "at least one non-empty owner is required",
            ));
        }
        self.kind.ensure_supported()
    }
}

fn metadata_error(id: &str, field: &str, reason: impl Into<String>) -> AppError {
    AppError::new(
        AppErrorCode::InvalidMetadata,
        "asset_metadata.validate",
        format!("{id}:{field}"),
        reason,
        "Correct the asset metadata and validate it again",
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptVariable {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PromptSpec {
    #[serde(default)]
    pub variables: Vec<PromptVariable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SkillSpec {
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub entrypoint: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AssetSpec {
    Prompt(PromptSpec),
    Skill(SkillSpec),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub metadata: AssetMetadata,
    pub spec: AssetSpec,
    pub body: String,
    pub source_files: Vec<PathBuf>,
}

impl Asset {
    pub fn reference(&self) -> Result<AssetRef, AppError> {
        AssetRef::new(self.metadata.kind, self.metadata.id.clone())
    }

    pub fn entry_path(&self) -> Option<&PathBuf> {
        self.source_files.first()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub path: PathBuf,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetSnapshot {
    pub asset: Asset,
    pub revision: RevisionSelector,
    pub commit: Option<String>,
    pub files: Vec<SnapshotFile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_asset_references() {
        let prompt = AssetRef::parse("prompt:code-review").unwrap();
        assert_eq!(prompt.kind, AssetKind::Prompt);
        assert_eq!(prompt.id, "code-review");
        assert_eq!(prompt.to_string(), "prompt:code-review");

        let skill = AssetRef::parse("skill:repo-onboarding-2").unwrap();
        assert_eq!(skill.kind, AssetKind::Skill);
    }

    #[test]
    fn rejects_invalid_asset_references() {
        for invalid in [
            "prompt",
            "prompt:",
            "prompt:CodeReview",
            "prompt:code_review",
            "prompt:-code-review",
            "prompt:code--review",
            "unknown:code-review",
            "prompt:code:review",
        ] {
            assert!(
                AssetRef::parse(invalid).is_err(),
                "expected {invalid} to be rejected"
            );
        }
    }

    #[test]
    fn mcp_parses_only_as_a_reserved_kind() {
        let reference = AssetRef::parse("mcp:filesystem").unwrap();
        assert_eq!(reference.kind, AssetKind::McpReserved);

        let error = reference.ensure_supported().unwrap_err();
        assert_eq!(error.code, AppErrorCode::ReservedAssetKind);
    }
}
