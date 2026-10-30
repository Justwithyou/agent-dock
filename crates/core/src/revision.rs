use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{AppError, AppErrorCode, AssetRef};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VersionTag {
    pub asset: AssetRef,
    pub name: String,
}

impl VersionTag {
    pub fn new(asset: AssetRef, name: impl Into<String>) -> Result<Self, AppError> {
        asset.ensure_supported()?;
        let name = name.into();
        validate_tag_name(&name)?;
        Ok(Self { asset, name })
    }

    pub fn parse(value: &str) -> Result<Self, AppError> {
        value.parse()
    }

    pub fn is_release_channel(&self) -> bool {
        self.name == "prod"
    }

    pub fn is_immutable_version(&self) -> bool {
        self.name.starts_with('v')
    }
}

impl fmt::Display for VersionTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "adock/{}/{}/{}",
            self.asset.kind, self.asset.id, self.name
        )
    }
}

impl FromStr for VersionTag {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut segments = value.split('/');
        let tag = match (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ) {
            (Some("adock"), Some(kind), Some(id), Some(name), None) => {
                let asset = AssetRef::parse(&format!("{kind}:{id}"))?;
                Self::new(asset, name)?
            }
            _ => {
                return Err(invalid_revision(value, "expected adock/<kind>/<id>/<name>"));
            }
        };
        Ok(tag)
    }
}

fn validate_tag_name(name: &str) -> Result<(), AppError> {
    let valid = !name.is_empty()
        && name
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !name.ends_with('.')
        && !name.contains("..");

    if valid {
        Ok(())
    } else {
        Err(invalid_revision(
            name,
            "version tag names must be non-empty Git-safe names such as v1.0.0 or prod",
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RevisionSelector {
    WorkingTree,
    Head,
    Commit(String),
    AssetTag(VersionTag),
}

impl RevisionSelector {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        value.parse()
    }
}

impl fmt::Display for RevisionSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkingTree => formatter.write_str("working-tree"),
            Self::Head => formatter.write_str("HEAD"),
            Self::Commit(sha) => formatter.write_str(sha),
            Self::AssetTag(tag) => tag.fmt(formatter),
        }
    }
}

impl FromStr for RevisionSelector {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "working" | "working-tree" => Ok(Self::WorkingTree),
            "HEAD" => Ok(Self::Head),
            _ if value.starts_with("adock/") => Ok(Self::AssetTag(value.parse()?)),
            _ if is_full_commit_sha(value) => Ok(Self::Commit(value.to_owned())),
            _ => Err(invalid_revision(
                value,
                "use working-tree, HEAD, a full commit SHA, or adock/<kind>/<id>/<name>",
            )),
        }
    }
}

fn is_full_commit_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn invalid_revision(value: &str, reason: impl Into<String>) -> AppError {
    AppError::new(
        AppErrorCode::InvalidRevision,
        "revision.parse",
        value,
        reason,
        "Choose an explicit supported revision selector",
    )
}

#[cfg(test)]
mod tests {
    use crate::AssetRef;

    use super::*;

    #[test]
    fn parses_revision_selectors() {
        assert_eq!(
            RevisionSelector::parse("working-tree").unwrap(),
            RevisionSelector::WorkingTree
        );
        assert_eq!(
            RevisionSelector::parse("HEAD").unwrap(),
            RevisionSelector::Head
        );
        assert_eq!(
            RevisionSelector::parse("0123456789abcdef0123456789abcdef01234567").unwrap(),
            RevisionSelector::Commit("0123456789abcdef0123456789abcdef01234567".to_owned())
        );
    }

    #[test]
    fn parses_and_formats_asset_version_tags() {
        let tag = VersionTag::parse("adock/prompt/code-review/v1.2.0").unwrap();
        assert_eq!(tag.asset, AssetRef::parse("prompt:code-review").unwrap());
        assert!(tag.is_immutable_version());
        assert!(!tag.is_release_channel());
        assert_eq!(tag.to_string(), "adock/prompt/code-review/v1.2.0");

        let prod = VersionTag::parse("adock/skill/repo-onboarding/prod").unwrap();
        assert!(prod.is_release_channel());
    }

    #[test]
    fn rejects_invalid_revision_and_tag_names() {
        for invalid in [
            "head",
            "abc123",
            "adock/prompt/code-review/",
            "adock/prompt/code-review/v1..0",
            "adock/prompt/code-review/v1/extra",
            "adock/mcp/filesystem/v1.0.0",
        ] {
            assert!(
                RevisionSelector::parse(invalid).is_err(),
                "expected {invalid} to be rejected"
            );
        }
    }
}
