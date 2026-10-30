use std::{collections::BTreeMap, path::PathBuf};

use adock_core::{
    AppError, AppErrorCode, AssetKind, AssetRef, DistributionPlan, RevisionSelector, TargetProfile,
    VersionTag, render_prompt,
};
use adock_infra::{DistributionService, load_workspace_config};
use serde::{Deserialize, Serialize};

use crate::{AdapterRegistry, GitRepository};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionRequest {
    pub reference: AssetRef,
    pub target: String,
    pub revision: Option<RevisionSelector>,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
    #[serde(default)]
    pub overwrite_managed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionTargetOptions {
    pub default_revision: String,
    pub targets: Vec<TargetProfile>,
}

pub struct DistributionWorkflow {
    workspace_root: PathBuf,
    registry: AdapterRegistry,
}

impl DistributionWorkflow {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            registry: AdapterRegistry::with_builtins(),
        }
    }

    pub fn target_options(
        &self,
        reference: &AssetRef,
    ) -> Result<DistributionTargetOptions, AppError> {
        reference.ensure_supported()?;
        let config = load_workspace_config(&self.workspace_root)?;
        let default_revision =
            VersionTag::new(reference.clone(), config.default_channel.clone())?.to_string();
        let mut targets = Vec::new();
        for target in config.targets.keys() {
            match self.registry.adapter(reference.kind, target) {
                Ok(_) => targets.push(config.target_profile(&self.workspace_root, target)?),
                Err(error) if error.code == AppErrorCode::UnsupportedAdapter => {}
                Err(error) => return Err(error),
            }
        }
        Ok(DistributionTargetOptions {
            default_revision,
            targets,
        })
    }

    pub fn create_plan(&self, request: &DistributionRequest) -> Result<DistributionPlan, AppError> {
        request.reference.ensure_supported()?;
        if request.reference.kind == AssetKind::Skill && !request.variables.is_empty() {
            return Err(AppError::new(
                AppErrorCode::InvalidInput,
                "distribution.variables",
                request.reference.to_string(),
                "Prompt variables cannot be supplied for a Skill",
                "Remove variable values or choose a Prompt asset",
            ));
        }

        let config = load_workspace_config(&self.workspace_root)?;
        let revision = match &request.revision {
            Some(revision) => revision.clone(),
            None => RevisionSelector::AssetTag(VersionTag::new(
                request.reference.clone(),
                config.default_channel.clone(),
            )?),
        };
        let target = config.target_profile(&self.workspace_root, &request.target)?;
        let adapter = self
            .registry
            .adapter(request.reference.kind, &request.target)?;
        let mut snapshot =
            GitRepository::open(&self.workspace_root)?.snapshot(&request.reference, &revision)?;
        if !request.variables.is_empty() {
            snapshot.asset.body = render_prompt(&snapshot.asset, Some(&request.variables))?.body;
        }
        DistributionService::create_plan(
            &snapshot,
            &target,
            adapter.as_ref(),
            request.overwrite_managed,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command};

    use adock_core::{AppErrorCode, AssetKind, DistributionPlanStatus};
    use tempfile::TempDir;

    use super::*;

    struct Fixture {
        _temp: TempDir,
        root: PathBuf,
        prod: String,
        head: String,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let root = temp.path().join("repo");
            fs::create_dir(&root).unwrap();
            git(&root, &["init", "-q"]);
            git(&root, &["config", "user.name", "AgentDock Workflow Tests"]);
            git(&root, &["config", "user.email", "tests@example.invalid"]);
            fs::write(
                root.join("adock.yaml"),
                "default_channel: prod\ntargets:\n  trae:\n    root: target/trae\n  cursor:\n    root: target/cursor\n  codex:\n    root: target/codex\n",
            )
            .unwrap();
            write_prompt(&root, "prod prompt");
            fs::create_dir_all(root.join("skills/onboard")).unwrap();
            fs::write(
                root.join("skills/onboard/SKILL.md"),
                "---\napiVersion: adock/v1alpha1\nkind: skill\nid: onboard\ntitle: Onboard\ndescription: Onboard a repository\nowners: [bob]\nspec:\n  triggers: []\n  dependencies: []\n---\nOnboard\n",
            )
            .unwrap();
            commit_all(&root, "prod assets");
            let prod = head(&root);
            git(&root, &["tag", "adock/prompt/review/prod"]);
            git(&root, &["tag", "adock/skill/onboard/prod"]);
            write_prompt(&root, "head prompt");
            commit_all(&root, "head prompt");
            let head = head(&root);
            Self {
                _temp: temp,
                root,
                prod,
                head,
            }
        }

        fn request(&self, kind: AssetKind, id: &str, target: &str) -> DistributionRequest {
            DistributionRequest {
                reference: AssetRef::new(kind, id).unwrap(),
                target: target.to_owned(),
                revision: None,
                variables: BTreeMap::new(),
                overwrite_managed: false,
            }
        }
    }

    fn write_prompt(root: &Path, marker: &str) {
        fs::create_dir_all(root.join("prompts")).unwrap();
        fs::write(
            root.join("prompts/review.md"),
            format!("---\napiVersion: adock/v1alpha1\nkind: prompt\nid: review\ntitle: Review\ndescription: Review code\nowners: [alice]\nspec:\n  variables:\n    - name: change\n      required: true\n    - name: tone\n      default: concise\n---\nReview {{{{change}}}} in a {{{{tone}}}} style.\n{marker}\n"),
        )
        .unwrap();
    }

    fn git(root: &Path, args: &[&str]) {
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
    }

    fn commit_all(root: &Path, message: &str) {
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", message]);
    }

    fn head(root: &Path) -> String {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn default_prod_and_explicit_revision_share_the_same_snapshot_pipeline() {
        let fixture = Fixture::new();
        let workflow = DistributionWorkflow::new(&fixture.root);
        let request = fixture.request(AssetKind::Prompt, "review", "trae");

        let default_plan = workflow.create_plan(&request).unwrap();
        assert_eq!(default_plan.commit, fixture.prod);
        assert_eq!(default_plan.status, DistributionPlanStatus::Ready);
        assert!(
            String::from_utf8(default_plan.files[0].content.clone())
                .unwrap()
                .contains("prod prompt")
        );

        let explicit = DistributionRequest {
            revision: Some(RevisionSelector::Head),
            ..request
        };
        let head_plan = workflow.create_plan(&explicit).unwrap();
        assert_eq!(head_plan.commit, fixture.head);
        assert!(
            String::from_utf8(head_plan.files[0].content.clone())
                .unwrap()
                .contains("head prompt")
        );
    }

    #[test]
    fn prompt_variables_render_deterministically_and_skill_variables_are_rejected() {
        let fixture = Fixture::new();
        let workflow = DistributionWorkflow::new(&fixture.root);
        let mut request = fixture.request(AssetKind::Prompt, "review", "trae");
        request
            .variables
            .insert("change".to_owned(), "PATCH".to_owned());
        let plan = workflow.create_plan(&request).unwrap();
        let content = String::from_utf8(plan.files[0].content.clone()).unwrap();
        assert!(content.contains("Review PATCH in a concise style."));
        assert!(!content.contains("{{change}}"));

        request
            .variables
            .insert("unknown".to_owned(), "secret".to_owned());
        assert_eq!(
            workflow.create_plan(&request).unwrap_err().code,
            AppErrorCode::ValidationFailed
        );

        let mut skill = fixture.request(AssetKind::Skill, "onboard", "codex");
        skill
            .variables
            .insert("change".to_owned(), "PATCH".to_owned());
        let error = workflow.create_plan(&skill).unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidInput);
        assert!(!error.reason.contains("PATCH"));
    }

    #[test]
    fn target_options_include_only_configured_supported_combinations() {
        let fixture = Fixture::new();
        let workflow = DistributionWorkflow::new(&fixture.root);
        let prompt = workflow
            .target_options(&AssetRef::new(AssetKind::Prompt, "review").unwrap())
            .unwrap();
        assert_eq!(prompt.default_revision, "adock/prompt/review/prod");
        assert_eq!(
            prompt
                .targets
                .iter()
                .map(|target| target.target.as_str())
                .collect::<Vec<_>>(),
            ["cursor", "trae"]
        );

        let skill = workflow
            .target_options(&AssetRef::new(AssetKind::Skill, "onboard").unwrap())
            .unwrap();
        assert_eq!(
            skill
                .targets
                .iter()
                .map(|target| target.target.as_str())
                .collect::<Vec<_>>(),
            ["codex", "trae"]
        );
    }
}
