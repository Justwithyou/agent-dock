use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use adock_core::{
    AppError, AppErrorCode, AssetKind, AssetRef, DistributionPlan, DistributionScope,
    RevisionSelector, TargetProfile, VersionTag, render_prompt,
};
use adock_infra::{DistributionService, load_workspace_config, save_local_target, user_home_dir};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraeEdition {
    Cn,
    International,
}

impl TraeEdition {
    fn directory(self) -> &'static str {
        match self {
            Self::Cn => ".trae-cn",
            Self::International => ".trae",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionTargetOptions {
    pub default_revision: String,
    pub default_revision_fallback: bool,
    pub targets: Vec<TargetProfile>,
    pub supported_targets: Vec<String>,
    pub trae_edition_options: Vec<TraeEdition>,
    pub trae_edition: Option<TraeEdition>,
}

struct StandardLocation {
    root: PathBuf,
    trae_edition_options: Vec<TraeEdition>,
    trae_edition: Option<TraeEdition>,
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
        let (default_revision, default_revision_fallback) =
            self.default_revision(reference, &config.default_channel)?;
        let mut targets = Vec::new();
        for target in config.targets.keys() {
            match self.registry.adapter(reference.kind, target) {
                Ok(_) => {
                    let profile = config.target_profile(&self.workspace_root, target)?;
                    targets.push(self.custom_target_profile(profile)?);
                }
                Err(error) if error.code == AppErrorCode::UnsupportedAdapter => {}
                Err(error) => return Err(error),
            }
        }
        Ok(DistributionTargetOptions {
            default_revision,
            default_revision_fallback,
            targets,
            supported_targets: self.registry.supported_targets(reference.kind),
            trae_edition_options: Vec::new(),
            trae_edition: None,
        })
    }

    pub fn scoped_target_options(
        &self,
        reference: &AssetRef,
        scope: DistributionScope,
        trae_edition: Option<TraeEdition>,
    ) -> Result<DistributionTargetOptions, AppError> {
        let home = user_home_dir();
        self.scoped_target_options_at(reference, scope, trae_edition, home.as_deref())
    }

    fn scoped_target_options_at(
        &self,
        reference: &AssetRef,
        scope: DistributionScope,
        trae_edition: Option<TraeEdition>,
        home: Option<&Path>,
    ) -> Result<DistributionTargetOptions, AppError> {
        reference.ensure_supported()?;
        let config = load_workspace_config(&self.workspace_root)?;
        let (default_revision, default_revision_fallback) =
            self.default_revision(reference, &config.default_channel)?;
        let supported_targets = self.registry.supported_targets(reference.kind);
        let mut targets = Vec::with_capacity(supported_targets.len());
        let mut trae_edition_options = Vec::new();
        let mut resolved_trae_edition = None;
        for target in &supported_targets {
            let location = self.standard_location(target, scope, trae_edition, home)?;
            if target == "trae" && scope == DistributionScope::User {
                trae_edition_options = location.trae_edition_options;
                resolved_trae_edition = location.trae_edition;
            }
            targets.push(TargetProfile::new(target, location.root)?);
        }
        Ok(DistributionTargetOptions {
            default_revision,
            default_revision_fallback,
            targets,
            supported_targets,
            trae_edition_options,
            trae_edition: resolved_trae_edition,
        })
    }

    pub fn configure_target(
        &self,
        reference: &AssetRef,
        target: &str,
        root: PathBuf,
    ) -> Result<DistributionTargetOptions, AppError> {
        reference.ensure_supported()?;
        self.registry.adapter(reference.kind, target)?;
        save_local_target(&self.workspace_root, target, root)?;
        self.target_options(reference)
    }

    pub fn create_plan(&self, request: &DistributionRequest) -> Result<DistributionPlan, AppError> {
        request.reference.ensure_supported()?;
        let config = load_workspace_config(&self.workspace_root)?;
        let revision = match &request.revision {
            Some(revision) => revision.clone(),
            None => RevisionSelector::AssetTag(VersionTag::new(
                request.reference.clone(),
                config.default_channel.clone(),
            )?),
        };
        let profile = config.target_profile(&self.workspace_root, &request.target)?;
        let target = self.custom_target_profile(profile)?;
        self.create_plan_for_profile(request, target, revision)
    }

    pub fn create_scoped_plan(
        &self,
        request: &DistributionRequest,
        scope: DistributionScope,
        trae_edition: Option<TraeEdition>,
    ) -> Result<DistributionPlan, AppError> {
        let home = user_home_dir();
        self.create_scoped_plan_at(request, scope, trae_edition, home.as_deref())
    }

    fn create_scoped_plan_at(
        &self,
        request: &DistributionRequest,
        scope: DistributionScope,
        trae_edition: Option<TraeEdition>,
        home: Option<&Path>,
    ) -> Result<DistributionPlan, AppError> {
        request.reference.ensure_supported()?;
        self.registry
            .adapter(request.reference.kind, &request.target)?;
        let location = self.standard_location(&request.target, scope, trae_edition, home)?;
        if request.target == "trae"
            && scope == DistributionScope::User
            && location.trae_edition.is_none()
        {
            return Err(AppError::new(
                AppErrorCode::InvalidConfig,
                "distribution.target.resolve",
                &request.target,
                "both Trae CN and international user directories exist",
                "Select a Trae edition before previewing user-level distribution",
            ));
        }
        let target = TargetProfile::new(&request.target, location.root)?;
        let revision = match &request.revision {
            Some(revision) => revision.clone(),
            None => {
                let config = load_workspace_config(&self.workspace_root)?;
                let (revision, _) =
                    self.default_revision(&request.reference, &config.default_channel)?;
                RevisionSelector::parse(&revision)?
            }
        };
        self.create_plan_for_profile(request, target, revision)
    }

    fn create_plan_for_profile(
        &self,
        request: &DistributionRequest,
        target: TargetProfile,
        revision: RevisionSelector,
    ) -> Result<DistributionPlan, AppError> {
        if request.reference.kind == AssetKind::Skill && !request.variables.is_empty() {
            return Err(AppError::new(
                AppErrorCode::InvalidInput,
                "distribution.variables",
                request.reference.to_string(),
                "Prompt variables cannot be supplied for a Skill",
                "Remove variable values or choose a Prompt asset",
            ));
        }
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

    fn custom_target_profile(&self, profile: TargetProfile) -> Result<TargetProfile, AppError> {
        let directory = self
            .registry
            .standard_directory(&profile.target, DistributionScope::Project)
            .ok_or_else(|| {
                AppError::new(
                    AppErrorCode::UnsupportedAdapter,
                    "distribution.target.resolve",
                    &profile.target,
                    "the target has no standard directory convention",
                    "Choose a supported target",
                )
            })?;
        if profile.root.file_name().and_then(|name| name.to_str()) == Some(directory) {
            return Ok(profile);
        }
        TargetProfile::new(&profile.target, profile.root.join(directory))
    }

    fn default_revision(
        &self,
        reference: &AssetRef,
        default_channel: &str,
    ) -> Result<(String, bool), AppError> {
        let default_revision =
            RevisionSelector::AssetTag(VersionTag::new(reference.clone(), default_channel)?);
        match GitRepository::open(&self.workspace_root)?.resolve(&default_revision) {
            Ok(_) => Ok((default_revision.to_string(), false)),
            Err(error) if error.code == AppErrorCode::UnknownRevision => {
                Ok((RevisionSelector::Head.to_string(), true))
            }
            Err(error) => Err(error),
        }
    }

    fn standard_location(
        &self,
        target: &str,
        scope: DistributionScope,
        trae_edition: Option<TraeEdition>,
        home: Option<&Path>,
    ) -> Result<StandardLocation, AppError> {
        let directory = self
            .registry
            .standard_directory(target, scope)
            .ok_or_else(|| {
                AppError::new(
                    AppErrorCode::UnsupportedAdapter,
                    "distribution.target.resolve",
                    target,
                    "the target has no standard directory convention",
                    "Choose a supported target or use an advanced custom directory",
                )
            })?;
        let base = match scope {
            DistributionScope::Project => self.workspace_root.clone(),
            DistributionScope::User => home
                .ok_or_else(|| {
                    AppError::new(
                        AppErrorCode::InvalidConfig,
                        "distribution.scope.resolve",
                        target,
                        "the user home directory is unavailable",
                        "Set the home directory or use project-level distribution",
                    )
                })?
                .to_path_buf(),
        };
        if target == "trae" && scope == DistributionScope::User {
            let cn_exists = base.join(".trae-cn").is_dir();
            let international_exists = base.join(".trae").is_dir();
            let (selected, options, resolved) =
                match (cn_exists, international_exists, trae_edition) {
                    (true, true, None) => (
                        TraeEdition::Cn,
                        vec![TraeEdition::Cn, TraeEdition::International],
                        None,
                    ),
                    (true, true, Some(edition)) => (
                        edition,
                        vec![TraeEdition::Cn, TraeEdition::International],
                        Some(edition),
                    ),
                    (false, true, None) => (
                        TraeEdition::International,
                        vec![TraeEdition::International],
                        Some(TraeEdition::International),
                    ),
                    (_, _, Some(edition)) => (edition, vec![edition], Some(edition)),
                    _ => (
                        TraeEdition::Cn,
                        vec![TraeEdition::Cn],
                        Some(TraeEdition::Cn),
                    ),
                };
            return Ok(StandardLocation {
                root: base.join(selected.directory()),
                trae_edition_options: options,
                trae_edition: resolved,
            });
        }
        Ok(StandardLocation {
            root: base.join(directory),
            trae_edition_options: Vec::new(),
            trae_edition: None,
        })
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
        assert!(!prompt.default_revision_fallback);
        assert_eq!(
            prompt.supported_targets,
            ["claude-code", "cursor", "trae", "workbuddy"]
        );
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

    #[test]
    fn custom_base_directory_resolves_to_the_targets_official_directory() {
        let fixture = Fixture::new();
        let workflow = DistributionWorkflow::new(&fixture.root);
        let reference = AssetRef::new(AssetKind::Skill, "onboard").unwrap();

        let options = workflow.target_options(&reference).unwrap();
        let trae = options
            .targets
            .iter()
            .find(|target| target.target == "trae")
            .unwrap();
        let expected = fixture.root.join("target/trae/.trae");
        assert_eq!(trae.root, expected);

        let plan = workflow
            .create_plan(&fixture.request(AssetKind::Skill, "onboard", "trae"))
            .unwrap();
        assert_eq!(plan.target_root, expected);
        assert!(
            plan.files
                .iter()
                .any(|file| file.target_path == expected.join("skills/onboard/SKILL.md"))
        );
    }

    #[test]
    fn custom_tool_directory_is_not_appended_twice() {
        let fixture = Fixture::new();
        let workflow = DistributionWorkflow::new(&fixture.root);
        let reference = AssetRef::new(AssetKind::Skill, "onboard").unwrap();
        let expected = fixture.root.join("target/custom-trae/.trae");

        let options = workflow
            .configure_target(&reference, "trae", expected.clone())
            .unwrap();
        let trae = options
            .targets
            .iter()
            .find(|target| target.target == "trae")
            .unwrap();
        assert_eq!(trae.root, expected);

        let plan = workflow
            .create_plan(&fixture.request(AssetKind::Skill, "onboard", "trae"))
            .unwrap();
        assert_eq!(plan.target_root, expected);
    }

    #[test]
    fn target_options_fall_back_to_head_when_default_channel_is_missing() {
        let fixture = Fixture::new();
        git(&fixture.root, &["tag", "-d", "adock/prompt/review/prod"]);
        let workflow = DistributionWorkflow::new(&fixture.root);
        let reference = AssetRef::new(AssetKind::Prompt, "review").unwrap();

        let options = workflow.target_options(&reference).unwrap();

        assert_eq!(options.default_revision, "HEAD");
        assert!(options.default_revision_fallback);
        let plan = workflow
            .create_plan(&DistributionRequest {
                revision: Some(RevisionSelector::parse(&options.default_revision).unwrap()),
                ..fixture.request(AssetKind::Prompt, "review", "trae")
            })
            .unwrap();
        assert_eq!(plan.commit, fixture.head);
    }

    #[test]
    fn project_scope_uses_standard_roots_instead_of_custom_configuration() {
        let fixture = Fixture::new();
        let workflow = DistributionWorkflow::new(&fixture.root);
        let reference = AssetRef::new(AssetKind::Prompt, "review").unwrap();

        let options = workflow
            .scoped_target_options_at(&reference, DistributionScope::Project, None, None)
            .unwrap();

        assert_eq!(options.targets.len(), 4);
        let trae = options
            .targets
            .iter()
            .find(|target| target.target == "trae")
            .unwrap();
        assert_eq!(trae.root, fixture.root.join(".trae"));
        let plan = workflow
            .create_scoped_plan_at(
                &fixture.request(AssetKind::Prompt, "review", "trae"),
                DistributionScope::Project,
                None,
                None,
            )
            .unwrap();
        assert_eq!(plan.target_root, fixture.root.join(".trae"));
    }

    #[test]
    fn user_scope_uses_standard_directories_below_the_injected_home() {
        let fixture = Fixture::new();
        let home = TempDir::new().unwrap();
        let workflow = DistributionWorkflow::new(&fixture.root);
        let reference = AssetRef::new(AssetKind::Prompt, "review").unwrap();

        let options = workflow
            .scoped_target_options_at(&reference, DistributionScope::User, None, Some(home.path()))
            .unwrap();

        let claude = options
            .targets
            .iter()
            .find(|target| target.target == "claude-code")
            .unwrap();
        assert_eq!(claude.root, home.path().join(".claude"));
        let plan = workflow
            .create_scoped_plan_at(
                &fixture.request(AssetKind::Prompt, "review", "claude-code"),
                DistributionScope::User,
                None,
                Some(home.path()),
            )
            .unwrap();
        assert_eq!(plan.target_root, home.path().join(".claude"));
    }

    #[test]
    fn trae_user_scope_detects_a_single_existing_edition() {
        let fixture = Fixture::new();
        let home = TempDir::new().unwrap();
        let workflow = DistributionWorkflow::new(&fixture.root);
        let reference = AssetRef::new(AssetKind::Prompt, "review").unwrap();
        fs::create_dir(home.path().join(".trae")).unwrap();
        let international = workflow
            .scoped_target_options_at(&reference, DistributionScope::User, None, Some(home.path()))
            .unwrap();
        assert_eq!(international.trae_edition, Some(TraeEdition::International));
        fs::remove_dir(home.path().join(".trae")).unwrap();
        fs::create_dir(home.path().join(".trae-cn")).unwrap();
        let cn = workflow
            .scoped_target_options_at(&reference, DistributionScope::User, None, Some(home.path()))
            .unwrap();
        assert_eq!(cn.trae_edition, Some(TraeEdition::Cn));
    }

    #[test]
    fn trae_user_scope_defaults_to_cn_when_no_edition_directory_exists() {
        let fixture = Fixture::new();
        let home = TempDir::new().unwrap();
        let workflow = DistributionWorkflow::new(&fixture.root);
        let reference = AssetRef::new(AssetKind::Prompt, "review").unwrap();

        let options = workflow
            .scoped_target_options_at(&reference, DistributionScope::User, None, Some(home.path()))
            .unwrap();

        assert_eq!(options.trae_edition, Some(TraeEdition::Cn));
        assert_eq!(options.trae_edition_options, [TraeEdition::Cn]);
        let trae = options
            .targets
            .iter()
            .find(|target| target.target == "trae")
            .unwrap();
        assert_eq!(trae.root, home.path().join(".trae-cn"));
    }

    #[test]
    fn trae_user_scope_requires_an_edition_when_both_directories_exist() {
        let fixture = Fixture::new();
        let home = TempDir::new().unwrap();
        fs::create_dir(home.path().join(".trae")).unwrap();
        fs::create_dir(home.path().join(".trae-cn")).unwrap();
        let workflow = DistributionWorkflow::new(&fixture.root);
        let reference = AssetRef::new(AssetKind::Prompt, "review").unwrap();
        let request = fixture.request(AssetKind::Prompt, "review", "trae");

        let options = workflow
            .scoped_target_options_at(&reference, DistributionScope::User, None, Some(home.path()))
            .unwrap();
        assert_eq!(options.trae_edition, None);
        assert_eq!(options.trae_edition_options.len(), 2);
        let error = workflow
            .create_scoped_plan_at(&request, DistributionScope::User, None, Some(home.path()))
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidConfig);

        let plan = workflow
            .create_scoped_plan_at(
                &request,
                DistributionScope::User,
                Some(TraeEdition::International),
                Some(home.path()),
            )
            .unwrap();
        assert_eq!(plan.target_root, home.path().join(".trae"));
    }
}
