use std::path::{Component, Path};

use adock_core::{
    AdapterFile, AdapterOutput, AppError, AppErrorCode, AssetKind, AssetSnapshot,
    DistributionAdapter,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptTarget {
    ClaudeCode,
    Cursor,
    Trae,
    WorkBuddy,
}

impl PromptTarget {
    fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Cursor => "cursor",
            Self::Trae => "trae",
            Self::WorkBuddy => "workbuddy",
        }
    }

    fn as_skill(self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Cursor)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PromptAdapter {
    target: PromptTarget,
}

impl PromptAdapter {
    pub fn claude_code() -> Self {
        Self {
            target: PromptTarget::ClaudeCode,
        }
    }

    pub fn cursor() -> Self {
        Self {
            target: PromptTarget::Cursor,
        }
    }

    pub fn trae() -> Self {
        Self {
            target: PromptTarget::Trae,
        }
    }

    pub fn workbuddy() -> Self {
        Self {
            target: PromptTarget::WorkBuddy,
        }
    }
}

impl DistributionAdapter for PromptAdapter {
    fn adapt(&self, snapshot: &AssetSnapshot) -> Result<AdapterOutput, AppError> {
        ensure_kind(snapshot, AssetKind::Prompt)?;
        let id = &snapshot.asset.metadata.id;
        let path = if self.target.as_skill() {
            format!("skills/{id}/SKILL.md")
        } else {
            format!("commands/{id}.md")
        };
        Ok(AdapterOutput {
            adapter_version: format!("{}-prompt/v1", self.target.name()),
            files: vec![AdapterFile {
                path,
                content: target_entry(snapshot, self.target.as_skill())?,
            }],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillTarget {
    ClaudeCode,
    Trae,
    Codex,
    WorkBuddy,
}

impl SkillTarget {
    fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Trae => "trae",
            Self::Codex => "codex",
            Self::WorkBuddy => "workbuddy",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SkillAdapter {
    target: SkillTarget,
}

impl SkillAdapter {
    pub fn claude_code() -> Self {
        Self {
            target: SkillTarget::ClaudeCode,
        }
    }

    pub fn trae() -> Self {
        Self {
            target: SkillTarget::Trae,
        }
    }

    pub fn codex() -> Self {
        Self {
            target: SkillTarget::Codex,
        }
    }

    pub fn workbuddy() -> Self {
        Self {
            target: SkillTarget::WorkBuddy,
        }
    }
}

impl DistributionAdapter for SkillAdapter {
    fn adapt(&self, snapshot: &AssetSnapshot) -> Result<AdapterOutput, AppError> {
        ensure_kind(snapshot, AssetKind::Skill)?;
        let reference = snapshot.asset.reference()?;
        let id = &reference.id;
        let entry_path = reference.entry_path();
        if !snapshot.files.iter().any(|file| file.path == entry_path) {
            return Err(adapter_error(
                snapshot,
                "the Skill snapshot has no SKILL.md entry",
            ));
        }
        let mut files = vec![AdapterFile {
            path: format!("skills/{id}/SKILL.md"),
            content: target_entry(snapshot, false)?,
        }];
        for file in snapshot.files.iter().filter(|file| file.path != entry_path) {
            let relative = file.path.strip_prefix(reference.root_path()).map_err(|_| {
                adapter_error(
                    snapshot,
                    format!(
                        "Skill resource '{}' escapes the asset directory",
                        file.path.display()
                    ),
                )
            })?;
            let relative = portable_relative_path(snapshot, relative)?;
            files.push(AdapterFile {
                path: format!("skills/{id}/{relative}"),
                content: file.content.clone(),
            });
        }
        Ok(AdapterOutput {
            adapter_version: format!("{}-skill/v1", self.target.name()),
            files,
        })
    }
}

#[derive(Serialize)]
struct TargetFrontmatter<'a> {
    name: &'a str,
    description: &'a str,
    #[serde(
        rename = "disable-model-invocation",
        skip_serializing_if = "Option::is_none"
    )]
    disable_model_invocation: Option<bool>,
}

fn target_entry(snapshot: &AssetSnapshot, manual_only: bool) -> Result<Vec<u8>, AppError> {
    let frontmatter = serde_yaml::to_string(&TargetFrontmatter {
        name: &snapshot.asset.metadata.id,
        description: &snapshot.asset.metadata.description,
        disable_model_invocation: manual_only.then_some(true),
    })
    .map_err(|error| {
        adapter_error(
            snapshot,
            format!("cannot encode target frontmatter: {error}"),
        )
    })?;
    Ok(format!("---\n{frontmatter}---\n{}", snapshot.asset.body).into_bytes())
}

fn portable_relative_path(snapshot: &AssetSnapshot, path: &Path) -> Result<String, AppError> {
    let mut components = Vec::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(adapter_error(
                snapshot,
                format!("invalid Skill resource path '{}'", path.display()),
            ));
        };
        let component = component.to_str().ok_or_else(|| {
            adapter_error(
                snapshot,
                format!("Skill resource path '{}' is not UTF-8", path.display()),
            )
        })?;
        components.push(component);
    }
    if components.is_empty() {
        return Err(adapter_error(
            snapshot,
            "Skill resource path cannot be empty",
        ));
    }
    Ok(components.join("/"))
}

fn ensure_kind(snapshot: &AssetSnapshot, expected: AssetKind) -> Result<(), AppError> {
    if snapshot.asset.metadata.kind != expected {
        return Err(adapter_error(
            snapshot,
            format!("expected a {expected} snapshot"),
        ));
    }
    Ok(())
}

fn adapter_error(snapshot: &AssetSnapshot, reason: impl Into<String>) -> AppError {
    AppError::new(
        AppErrorCode::InvalidAsset,
        "distribution.adapter",
        snapshot.asset.metadata.id.clone(),
        reason,
        "Use a valid snapshot with the asset kind expected by the adapter",
    )
}

#[cfg(test)]
mod tests {
    use adock_core::{RevisionSelector, SnapshotFile, parse_asset_with_sources};
    use std::path::PathBuf;

    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn prompt_snapshot() -> AssetSnapshot {
        let path = PathBuf::from("prompts/review.md");
        let input = "---\napiVersion: adock/v1alpha1\nkind: prompt\nid: review\ntitle: Review\ndescription: Review code\nowners: [alice]\nspec:\n  variables:\n    - name: path\n      required: true\n---\nReview {{path}}\n";
        let asset = parse_asset_with_sources(&path, input, vec![path.clone()]).unwrap();
        AssetSnapshot {
            asset,
            revision: RevisionSelector::Head,
            commit: Some(COMMIT.to_owned()),
            files: vec![SnapshotFile {
                path,
                content: input.as_bytes().to_vec(),
            }],
        }
    }

    fn skill_snapshot() -> AssetSnapshot {
        let entry = PathBuf::from("skills/onboard/SKILL.md");
        let guide = PathBuf::from("skills/onboard/references/guide.txt");
        let script = PathBuf::from("skills/onboard/scripts/run.sh");
        let binary = PathBuf::from("skills/onboard/blob.bin");
        let input = "---\napiVersion: adock/v1alpha1\nkind: skill\nid: onboard\ntitle: Onboard\ndescription: Onboard a repository\nowners: [bob]\nspec:\n  triggers: []\n  dependencies: []\n---\nRead the guide\n";
        let paths = vec![entry.clone(), guide.clone(), script.clone(), binary.clone()];
        let asset = parse_asset_with_sources(&entry, input, paths).unwrap();
        AssetSnapshot {
            asset,
            revision: RevisionSelector::Head,
            commit: Some(COMMIT.to_owned()),
            files: vec![
                SnapshotFile {
                    path: entry,
                    content: input.as_bytes().to_vec(),
                },
                SnapshotFile {
                    path: guide,
                    content: b"Guide\n".to_vec(),
                },
                SnapshotFile {
                    path: script,
                    content: b"echo not-executed\n".to_vec(),
                },
                SnapshotFile {
                    path: binary,
                    content: vec![0, 159, 146, 150],
                },
            ],
        }
    }

    #[test]
    fn prompt_adapters_match_golden_files() {
        let cases: Vec<(PromptAdapter, &str, &str)> = vec![
            (
                PromptAdapter::claude_code(),
                "skills/review/SKILL.md",
                include_str!("../../../tests/golden/adapters/claude-code-prompt.md"),
            ),
            (
                PromptAdapter::cursor(),
                "skills/review/SKILL.md",
                include_str!("../../../tests/golden/adapters/cursor-prompt.md"),
            ),
            (
                PromptAdapter::trae(),
                "commands/review.md",
                include_str!("../../../tests/golden/adapters/trae-prompt.md"),
            ),
            (
                PromptAdapter::workbuddy(),
                "commands/review.md",
                include_str!("../../../tests/golden/adapters/workbuddy-prompt.md"),
            ),
        ];
        for (adapter, path, golden) in cases {
            let output = adapter.adapt(&prompt_snapshot()).unwrap();
            assert_eq!(output.files.len(), 1);
            assert_eq!(output.files[0].path, path);
            assert_eq!(
                String::from_utf8(output.files[0].content.clone()).unwrap(),
                golden
            );
        }
    }

    #[test]
    fn skill_adapters_match_golden_files_and_preserve_resources_without_execution() {
        let cases: Vec<(SkillAdapter, &str)> = vec![
            (
                SkillAdapter::claude_code(),
                include_str!("../../../tests/golden/adapters/claude-code-skill.md"),
            ),
            (
                SkillAdapter::trae(),
                include_str!("../../../tests/golden/adapters/trae-skill.md"),
            ),
            (
                SkillAdapter::codex(),
                include_str!("../../../tests/golden/adapters/codex-skill.md"),
            ),
            (
                SkillAdapter::workbuddy(),
                include_str!("../../../tests/golden/adapters/workbuddy-skill.md"),
            ),
        ];
        for (adapter, golden) in cases {
            let output = adapter.adapt(&skill_snapshot()).unwrap();
            assert_eq!(output.files.len(), 4);
            assert_eq!(output.files[0].path, "skills/onboard/SKILL.md");
            assert_eq!(
                String::from_utf8(output.files[0].content.clone()).unwrap(),
                golden
            );
            assert_eq!(output.files[1].path, "skills/onboard/references/guide.txt");
            assert_eq!(output.files[1].content, b"Guide\n");
            assert_eq!(output.files[2].path, "skills/onboard/scripts/run.sh");
            assert_eq!(output.files[2].content, b"echo not-executed\n");
            assert_eq!(output.files[3].path, "skills/onboard/blob.bin");
            assert_eq!(output.files[3].content, [0, 159, 146, 150]);
        }
    }

    #[test]
    fn adapters_reject_the_wrong_asset_kind() {
        let error = PromptAdapter::trae().adapt(&skill_snapshot()).unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidAsset);
        let error = SkillAdapter::codex().adapt(&prompt_snapshot()).unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidAsset);
    }

    #[test]
    fn historical_asset_tag_distributes_without_checkout_or_branch_changes() {
        use std::process::Command;

        use adock_core::{AssetRef, DistributionPlanStatus, TargetProfile};
        use adock_infra::DistributionService;
        use tempfile::TempDir;

        use crate::{AdapterRegistry, GitRepository};

        fn git(root: &Path, args: &[&str]) -> String {
            let output = Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }

        let temp = TempDir::new().unwrap();
        let root = temp.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.name", "Agent Dock Tests"]);
        git(root, &["config", "user.email", "tests@example.invalid"]);
        std::fs::create_dir_all(root.join("prompts")).unwrap();
        let document = |body: &str| {
            format!(
                "---\napiVersion: adock/v1alpha1\nkind: prompt\nid: review\ntitle: Review\ndescription: Review code\nowners: [alice]\nspec:\n  variables: []\n---\n{body}\n"
            )
        };
        std::fs::write(
            root.join("prompts/review.md"),
            document("Historical review"),
        )
        .unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-q", "-m", "historical prompt"]);
        let historical = git(root, &["rev-parse", "HEAD"]);
        git(root, &["tag", "adock/prompt/review/v1.0.0"]);
        std::fs::write(root.join("prompts/review.md"), document("Current review")).unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-q", "-m", "current prompt"]);
        let current = git(root, &["rev-parse", "HEAD"]);
        let branch = git(root, &["branch", "--show-current"]);

        let repository = GitRepository::open(root).unwrap();
        let reference = AssetRef::parse("prompt:review").unwrap();
        let selector = adock_core::RevisionSelector::parse("adock/prompt/review/v1.0.0").unwrap();
        let snapshot = repository.snapshot(&reference, &selector).unwrap();
        let registry = AdapterRegistry::with_builtins();
        let adapter = registry.adapter(AssetKind::Prompt, "trae").unwrap();
        let target = TargetProfile::new("trae", root.join("target")).unwrap();
        let plan =
            DistributionService::create_plan(&snapshot, &target, adapter.as_ref(), false).unwrap();
        assert_eq!(plan.status, DistributionPlanStatus::Ready);
        assert_eq!(plan.commit, historical);
        DistributionService::execute(&plan).unwrap();

        assert_eq!(
            std::fs::read_to_string(target.root.join("commands/review.md")).unwrap(),
            "---\nname: review\ndescription: Review code\n---\nHistorical review\n"
        );
        assert_eq!(git(root, &["rev-parse", "HEAD"]), current);
        assert_eq!(git(root, &["branch", "--show-current"]), branch);
        assert_eq!(
            std::fs::read_to_string(root.join("prompts/review.md")).unwrap(),
            document("Current review")
        );
    }
}
