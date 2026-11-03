use std::{collections::BTreeMap, path::Path};

use adock_adapters::{
    DistributionRequest, DistributionTargetOptions, DistributionWorkflow, GitLogEntry,
    GitRepository, TraeEdition,
};
use adock_core::{
    AppError, AppErrorCode, Asset, AssetDiff, AssetRef, DistributionExecution, DistributionPlan,
    DistributionScope, RevisionSelector, ValidationReport, VersionTag, WorkspaceInfo,
    WorkspaceService,
};
use adock_infra::{
    AssetFilter, AssetService, DistributionService, FileSystemWorkspaceDiscovery,
    WorkspaceInitializer, load_workspace_config,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct DesktopAssetListItem {
    asset: Option<Asset>,
    error: Option<AppError>,
}

#[tauri::command]
fn discover_workspace(path: Option<String>) -> Result<WorkspaceInfo, AppError> {
    let service = WorkspaceService::new(FileSystemWorkspaceDiscovery);
    let info = service.discover(path.as_deref().map(Path::new))?;
    load_workspace_config(&info.root)?;
    Ok(info)
}

#[tauri::command]
fn initialize_workspace(path: String) -> Result<WorkspaceInfo, AppError> {
    let info = WorkspaceInitializer::initialize(Path::new(&path))?;
    load_workspace_config(&info.root)?;
    Ok(info)
}

#[tauri::command]
fn list_assets(
    path: String,
    filter: Option<AssetFilter>,
) -> Result<Vec<DesktopAssetListItem>, AppError> {
    let service = AssetService::new(path);
    let result = service.list(&filter.unwrap_or_default())?;
    let mut entries = Vec::with_capacity(result.assets.len() + result.issues.len());

    for item in result.assets {
        let asset = service.show(&item.asset)?;
        entries.push(DesktopAssetListItem {
            asset: Some(asset),
            error: None,
        });
    }
    for issue in result.issues {
        entries.push(DesktopAssetListItem {
            asset: None,
            error: Some(issue.error),
        });
    }
    Ok(entries)
}

#[tauri::command]
fn show_asset(path: String, reference: String) -> Result<Asset, AppError> {
    AssetService::new(path).show(&AssetRef::parse(&reference)?)
}

#[tauri::command]
fn validate_workspace(path: String) -> Result<ValidationReport, AppError> {
    AssetService::new(path).validate(None)
}

#[derive(Debug, Serialize)]
struct DesktopVersionTag {
    name: String,
    full_name: String,
    commit: String,
    immutable: bool,
    release_channel: bool,
}

#[derive(Debug, Serialize)]
struct DesktopAssetHistory {
    commits: Vec<GitLogEntry>,
    tags: Vec<DesktopVersionTag>,
}

#[tauri::command]
fn get_asset_history(
    path: String,
    reference: String,
    limit: Option<usize>,
) -> Result<DesktopAssetHistory, AppError> {
    let reference = AssetRef::parse(&reference)?;
    let repository = GitRepository::open(path)?;
    let commits = repository.log(&reference, limit)?;
    let tags = repository
        .list_tags(Some(&reference))?
        .into_iter()
        .map(|tag| {
            let commit = repository
                .resolve(&RevisionSelector::AssetTag(tag.clone()))?
                .commit
                .expect("an asset tag always resolves to a commit");
            Ok(DesktopVersionTag {
                name: tag.name.clone(),
                full_name: tag.to_string(),
                commit,
                immutable: tag.is_immutable_version(),
                release_channel: tag.is_release_channel(),
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(DesktopAssetHistory { commits, tags })
}

#[tauri::command]
fn diff_asset(
    path: String,
    reference: String,
    from: String,
    to: String,
) -> Result<AssetDiff, AppError> {
    GitRepository::open(path)?.diff(
        &AssetRef::parse(&reference)?,
        &RevisionSelector::parse(&from)?,
        &RevisionSelector::parse(&to)?,
    )
}

#[tauri::command]
fn create_version_tag(
    path: String,
    reference: String,
    name: String,
    target: String,
    move_tag: bool,
    confirmed: bool,
) -> Result<VersionTag, AppError> {
    let tag = VersionTag::new(AssetRef::parse(&reference)?, name)?;
    GitRepository::open(path)?.create_tag(
        &tag,
        &RevisionSelector::parse(&target)?,
        move_tag,
        confirmed,
    )
}

#[tauri::command]
fn preview_rollback(
    path: String,
    reference: String,
    target: String,
) -> Result<AssetDiff, AppError> {
    GitRepository::open(path)?.rollback_preview(
        &AssetRef::parse(&reference)?,
        &RevisionSelector::parse(&target)?,
    )
}

#[tauri::command]
fn rollback_asset(
    path: String,
    reference: String,
    target: String,
    confirmed: bool,
) -> Result<AssetDiff, AppError> {
    GitRepository::open(path)?.rollback(
        &AssetRef::parse(&reference)?,
        &RevisionSelector::parse(&target)?,
        confirmed,
    )
}

#[tauri::command]
fn get_distribution_options(
    path: String,
    reference: String,
) -> Result<DistributionTargetOptions, AppError> {
    let root = discover_workspace(Some(path))?.root;
    DistributionWorkflow::new(root).target_options(&AssetRef::parse(&reference)?)
}

#[tauri::command]
fn get_scoped_distribution_options(
    path: String,
    reference: String,
    scope: DistributionScope,
    trae_edition: Option<TraeEdition>,
) -> Result<DistributionTargetOptions, AppError> {
    let root = discover_workspace(Some(path))?.root;
    DistributionWorkflow::new(root).scoped_target_options(
        &AssetRef::parse(&reference)?,
        scope,
        trae_edition,
    )
}

#[tauri::command]
fn configure_distribution_target(
    path: String,
    reference: String,
    target: String,
    root_path: String,
) -> Result<DistributionTargetOptions, AppError> {
    let root = discover_workspace(Some(path))?.root;
    DistributionWorkflow::new(root).configure_target(
        &AssetRef::parse(&reference)?,
        &target,
        root_path.into(),
    )
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn preview_scoped_distribution(
    path: String,
    reference: String,
    target: String,
    scope: DistributionScope,
    trae_edition: Option<TraeEdition>,
    revision: Option<String>,
    variables: BTreeMap<String, String>,
    overwrite_managed: bool,
) -> Result<DistributionPlan, AppError> {
    let revision = revision
        .filter(|revision| !revision.trim().is_empty())
        .map(|revision| RevisionSelector::parse(&revision))
        .transpose()?;
    let root = discover_workspace(Some(path))?.root;
    DistributionWorkflow::new(root).create_scoped_plan(
        &DistributionRequest {
            reference: AssetRef::parse(&reference)?,
            target,
            revision,
            variables,
            overwrite_managed,
        },
        scope,
        trae_edition,
    )
}

#[tauri::command]
fn pick_directory(start_path: Option<String>) -> Option<String> {
    let mut dialog = rfd::FileDialog::new();
    if let Some(start_path) = start_path.filter(|path| !path.trim().is_empty()) {
        dialog = dialog.set_directory(start_path);
    }
    dialog
        .pick_folder()
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
fn preview_distribution(
    path: String,
    reference: String,
    target: String,
    revision: Option<String>,
    variables: BTreeMap<String, String>,
    overwrite_managed: bool,
) -> Result<DistributionPlan, AppError> {
    let revision = revision
        .filter(|revision| !revision.trim().is_empty())
        .map(|revision| RevisionSelector::parse(&revision))
        .transpose()?;
    let root = discover_workspace(Some(path))?.root;
    DistributionWorkflow::new(root).create_plan(&DistributionRequest {
        reference: AssetRef::parse(&reference)?,
        target,
        revision,
        variables,
        overwrite_managed,
    })
}

#[tauri::command]
fn execute_distribution(
    plan: DistributionPlan,
    confirmed: bool,
) -> Result<DistributionExecution, AppError> {
    if !confirmed {
        return Err(AppError::new(
            AppErrorCode::ConfirmationRequired,
            "desktop.distribution.execute",
            plan.asset.to_string(),
            "distribution execution requires explicit confirmation",
            "Review the current plan and confirm before writing target files",
        ));
    }
    DistributionService::execute(&plan)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            discover_workspace,
            initialize_workspace,
            list_assets,
            show_asset,
            validate_workspace,
            get_asset_history,
            diff_asset,
            create_version_tag,
            preview_rollback,
            rollback_asset,
            get_distribution_options,
            get_scoped_distribution_options,
            configure_distribution_target,
            pick_directory,
            preview_distribution,
            preview_scoped_distribution,
            execute_distribution
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the AgentDock desktop application");
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command as ProcessCommand,
    };

    use adock_core::{
        AppErrorCode, DistributionConflict, DistributionExecutionStatus, DistributionPlanStatus,
        PlannedFileStatus,
    };
    use assert_cmd::Command;

    use super::*;

    struct VersionFixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        first: String,
        second: String,
    }

    impl VersionFixture {
        fn new() -> Self {
            let temp = tempfile::TempDir::new().unwrap();
            let root = temp.path().join("repo");
            fs::create_dir(&root).unwrap();
            git(&root, &["init", "-q"]);
            git(&root, &["config", "user.name", "AgentDock Desktop Tests"]);
            git(&root, &["config", "user.email", "tests@example.invalid"]);
            fs::write(
                root.join("adock.yaml"),
                "default_channel: prod\ntargets:\n  trae:\n    root: target/trae\n  codex:\n    root: target/codex\n",
            )
            .unwrap();
            write_prompt(&root, "first prompt");
            write_skill(&root, "first skill");
            fs::write(root.join("skills/onboard/obsolete.txt"), "remove me\n").unwrap();
            commit_all(&root, "first assets");
            let first = head(&root);

            write_prompt(&root, "second prompt");
            write_skill(&root, "second skill");
            fs::remove_file(root.join("skills/onboard/obsolete.txt")).unwrap();
            fs::create_dir_all(root.join("skills/onboard/references")).unwrap();
            fs::write(
                root.join("skills/onboard/references/guide.md"),
                "second guide\n",
            )
            .unwrap();
            commit_all(&root, "second assets");
            let second = head(&root);

            Self {
                _temp: temp,
                root,
                first,
                second,
            }
        }

        fn path(&self) -> String {
            self.root.display().to_string()
        }

        fn cli(&self) -> Command {
            let mut command = Command::cargo_bin("adock").unwrap();
            command.arg("--workspace").arg(&self.root);
            command
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let output = ProcessCommand::new("git")
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
        let output = ProcessCommand::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn write_prompt(root: &Path, marker: &str) {
        fs::create_dir_all(root.join("prompts")).unwrap();
        fs::write(
            root.join("prompts/review.md"),
            format!(
                "---\napiVersion: adock/v1alpha1\nkind: prompt\nid: review\ntitle: Review\ndescription: Review code\nowners: [alice]\nspec:\n  variables: []\n---\n{marker}\n"
            ),
        )
        .unwrap();
    }

    fn write_skill(root: &Path, marker: &str) {
        fs::create_dir_all(root.join("skills/onboard/scripts")).unwrap();
        fs::write(
            root.join("skills/onboard/SKILL.md"),
            format!(
                "---\napiVersion: adock/v1alpha1\nkind: skill\nid: onboard\ntitle: Onboard\ndescription: Onboard a repository\nowners: [bob]\nspec:\n  triggers: [new repository]\n  dependencies: []\n  entrypoint: scripts/run.sh\n---\n{marker}\n"
            ),
        )
        .unwrap();
        fs::write(
            root.join("skills/onboard/scripts/run.sh"),
            format!("echo {marker}\n"),
        )
        .unwrap();
    }

    fn write_renderable_prompt(root: &Path, marker: &str) {
        fs::write(
            root.join("prompts/review.md"),
            format!(
                "---\napiVersion: adock/v1alpha1\nkind: prompt\nid: review\ntitle: Review\ndescription: Review code\nowners: [alice]\nspec:\n  variables:\n    - name: change\n      required: true\n    - name: tone\n      default: concise\n---\nReview {{{{change}}}} in a {{{{tone}}}} style.\n{marker}\n"
            ),
        )
        .unwrap();
    }

    fn create_prod(fixture: &VersionFixture, reference: &str, target: &str) {
        create_version_tag(
            fixture.path(),
            reference.to_owned(),
            "prod".to_owned(),
            target.to_owned(),
            true,
            true,
        )
        .unwrap();
    }

    fn assert_cli_matches_plan(output: &[u8], plan: &DistributionPlan) {
        let output = String::from_utf8_lossy(output);
        for expected in [
            format!("Distribution plan: {} -> {}", plan.asset, plan.target),
            format!("Source: {}", plan.source),
            format!("Commit: {}", plan.commit),
            format!("Target root: {}", plan.target_root.display()),
            format!("Adapter: {}", plan.adapter_version),
        ] {
            assert!(output.contains(&expected), "missing {expected:?}\n{output}");
        }
        for file in &plan.files {
            let status = match file.status {
                PlannedFileStatus::Added => "added",
                PlannedFileStatus::Updated => "updated",
                PlannedFileStatus::Unchanged => "unchanged",
                PlannedFileStatus::Conflict => "conflict",
            };
            let mut expected = format!("  {status} | {}", file.target_path.display());
            if let Some(conflict) = file.conflict {
                expected.push_str(match conflict {
                    DistributionConflict::UnmanagedFile => " | unmanaged_file",
                    DistributionConflict::LocallyModified => " | locally_modified",
                });
            }
            expected.push_str(&format!(" | sha256:{}", file.checksum));
            assert!(output.contains(&expected), "missing {expected:?}\n{output}");
        }
    }

    #[test]
    fn asset_commands_share_core_services_and_isolate_broken_assets() {
        let temp = tempfile::TempDir::new().unwrap();
        git(temp.path(), &["init", "-q"]);
        fs::write(temp.path().join("adock.yaml"), "targets: {}\n").unwrap();
        fs::create_dir_all(temp.path().join("prompts")).unwrap();
        fs::write(
            temp.path().join("prompts/review.md"),
            "---\napiVersion: adock/v1alpha1\nkind: prompt\nid: review\ntitle: Review\ndescription: Review code\nowners: [alice]\nspec:\n  variables: []\n---\nReview code\n",
        )
        .unwrap();
        fs::write(temp.path().join("prompts/broken.md"), "broken").unwrap();

        let entries = list_assets(temp.path().display().to_string(), None).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries.iter().filter(|entry| entry.asset.is_some()).count(),
            1
        );
        assert_eq!(
            entries.iter().filter(|entry| entry.error.is_some()).count(),
            1
        );

        let asset = show_asset(
            temp.path().display().to_string(),
            "prompt:review".to_owned(),
        )
        .unwrap();
        assert_eq!(asset.metadata.id, "review");

        let report = validate_workspace(temp.path().display().to_string()).unwrap();
        assert!(!report.valid);
        assert_eq!(report.asset_count, 1);

        let filter = AssetFilter {
            query: Some("ALICE".to_owned()),
            ..AssetFilter::default()
        };
        let filtered = list_assets(temp.path().display().to_string(), Some(filter)).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].asset.as_ref().unwrap().metadata.id, "review");
        let output = Command::cargo_bin("adock")
            .unwrap()
            .arg("--workspace")
            .arg(temp.path())
            .args(["list", "--query", "ALICE"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("prompt:review"));
        assert!(!output.contains("broken.md"));
    }

    #[test]
    fn prompt_versions_cross_cli_and_preserve_error_categories() {
        let fixture = VersionFixture::new();
        let history =
            get_asset_history(fixture.path(), "prompt:review".to_owned(), Some(10)).unwrap();
        assert_eq!(history.commits.len(), 2);
        assert!(history.tags.is_empty());

        let diff = diff_asset(
            fixture.path(),
            "prompt:review".to_owned(),
            fixture.first.clone(),
            fixture.second.clone(),
        )
        .unwrap();
        assert_eq!(diff.files.len(), 1);
        assert!(
            diff.files[0]
                .text_diff
                .as_ref()
                .unwrap()
                .contains("+second prompt")
        );

        create_version_tag(
            fixture.path(),
            "prompt:review".to_owned(),
            "v1.0.0".to_owned(),
            fixture.first.clone(),
            false,
            false,
        )
        .unwrap();
        let history = get_asset_history(fixture.path(), "prompt:review".to_owned(), None).unwrap();
        assert_eq!(history.tags.len(), 1);
        assert_eq!(history.tags[0].commit, fixture.first);
        assert!(history.tags[0].immutable);

        let output = fixture
            .cli()
            .args([
                "show",
                "prompt",
                "review",
                "--ref",
                "adock/prompt/review/v1.0.0",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        assert!(String::from_utf8_lossy(&output).contains("first prompt"));

        let error = create_version_tag(
            fixture.path(),
            "prompt:review".to_owned(),
            "v1.0.0".to_owned(),
            fixture.second.clone(),
            false,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::TagAlreadyExists);

        let error = create_version_tag(
            fixture.path(),
            "prompt:review".to_owned(),
            "prod".to_owned(),
            fixture.second.clone(),
            false,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::ConfirmationRequired);
        create_version_tag(
            fixture.path(),
            "prompt:review".to_owned(),
            "prod".to_owned(),
            fixture.second.clone(),
            true,
            true,
        )
        .unwrap();
        fixture
            .cli()
            .args([
                "show",
                "prompt",
                "review",
                "--ref",
                "adock/prompt/review/prod",
            ])
            .assert()
            .success();

        fixture
            .cli()
            .args([
                "rollback",
                "prompt",
                "review",
                "--to",
                "adock/prompt/review/v1.0.0",
                "--yes",
            ])
            .assert()
            .success();
        let restored = show_asset(fixture.path(), "prompt:review".to_owned()).unwrap();
        assert_eq!(restored.body, "first prompt\n");
        let diff = diff_asset(
            fixture.path(),
            "prompt:review".to_owned(),
            "HEAD".to_owned(),
            "working-tree".to_owned(),
        )
        .unwrap();
        assert!(
            diff.files[0]
                .text_diff
                .as_ref()
                .unwrap()
                .contains("+first prompt")
        );

        let unknown = "0000000000000000000000000000000000000000";
        let error = diff_asset(
            fixture.path(),
            "prompt:review".to_owned(),
            unknown.to_owned(),
            "HEAD".to_owned(),
        )
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::UnknownRevision);
        let output = fixture
            .cli()
            .args([
                "diff", "prompt", "review", "--from", unknown, "--to", "HEAD",
            ])
            .assert()
            .failure()
            .get_output()
            .stderr
            .clone();
        assert!(String::from_utf8_lossy(&output).contains("error[unknown_revision]"));
    }

    #[test]
    fn skill_rollback_restores_the_whole_directory_for_cli_diff() {
        let fixture = VersionFixture::new();
        let history = get_asset_history(fixture.path(), "skill:onboard".to_owned(), None).unwrap();
        assert_eq!(history.commits.len(), 2);

        let diff = diff_asset(
            fixture.path(),
            "skill:onboard".to_owned(),
            fixture.first.clone(),
            fixture.second.clone(),
        )
        .unwrap();
        assert!(diff.files.iter().any(|file| {
            file.path == Path::new("skills/onboard/obsolete.txt")
                && file.change == adock_core::ChangeKind::Removed
        }));
        assert!(diff.files.iter().any(|file| {
            file.path == Path::new("skills/onboard/references/guide.md")
                && file.change == adock_core::ChangeKind::Added
        }));

        create_version_tag(
            fixture.path(),
            "skill:onboard".to_owned(),
            "v1.0.0".to_owned(),
            fixture.first.clone(),
            false,
            false,
        )
        .unwrap();
        let target = "adock/skill/onboard/v1.0.0".to_owned();
        let preview =
            preview_rollback(fixture.path(), "skill:onboard".to_owned(), target.clone()).unwrap();
        assert!(!preview.is_empty());
        let error = rollback_asset(
            fixture.path(),
            "skill:onboard".to_owned(),
            target.clone(),
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::ConfirmationRequired);
        assert!(!fixture.root.join("skills/onboard/obsolete.txt").exists());

        rollback_asset(fixture.path(), "skill:onboard".to_owned(), target, true).unwrap();
        assert!(fixture.root.join("skills/onboard/obsolete.txt").is_file());
        assert!(
            !fixture
                .root
                .join("skills/onboard/references/guide.md")
                .exists()
        );
        assert!(
            fs::read_to_string(fixture.root.join("prompts/review.md"))
                .unwrap()
                .ends_with("second prompt\n")
        );

        let output = fixture
            .cli()
            .args([
                "diff",
                "skill",
                "onboard",
                "--from",
                "HEAD",
                "--to",
                "working-tree",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("obsolete.txt"));
        assert!(output.contains("guide.md"), "{output}");

        let error = preview_rollback(
            fixture.path(),
            "skill:onboard".to_owned(),
            fixture.first.clone(),
        )
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::DirtyAsset);
    }

    #[test]
    fn desktop_can_configure_a_supported_local_target_without_changing_shared_config() {
        let fixture = VersionFixture::new();
        let shared = fs::read_to_string(fixture.root.join("adock.yaml")).unwrap();

        let options = configure_distribution_target(
            fixture.path(),
            "prompt:review".to_owned(),
            "workbuddy".to_owned(),
            "target/local-workbuddy".to_owned(),
        )
        .unwrap();

        assert!(
            options
                .targets
                .iter()
                .any(|target| target.target == "workbuddy")
        );
        assert_eq!(
            fs::read_to_string(fixture.root.join("adock.yaml")).unwrap(),
            shared
        );
        let local = fs::read_to_string(fixture.root.join("adock.local.yaml")).unwrap();
        assert!(local.contains("workbuddy:"));
        assert!(local.contains("root: target/local-workbuddy"));
        let error = configure_distribution_target(
            fixture.path(),
            "prompt:review".to_owned(),
            "codex".to_owned(),
            "target/codex-prompt".to_owned(),
        )
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::UnsupportedAdapter);
    }

    #[test]
    fn scoped_distribution_commands_resolve_project_directories_without_custom_targets() {
        let fixture = VersionFixture::new();
        let workspace_root = fixture.root.canonicalize().unwrap();

        let options = get_scoped_distribution_options(
            fixture.path(),
            "prompt:review".to_owned(),
            DistributionScope::Project,
            None,
        )
        .unwrap();
        assert_eq!(options.targets.len(), 4);
        let trae = options
            .targets
            .iter()
            .find(|target| target.target == "trae")
            .unwrap();
        assert_eq!(trae.root, workspace_root.join(".trae"));

        let plan = preview_scoped_distribution(
            fixture.path(),
            "prompt:review".to_owned(),
            "trae".to_owned(),
            DistributionScope::Project,
            None,
            Some("HEAD".to_owned()),
            BTreeMap::new(),
            false,
        )
        .unwrap();
        assert_eq!(plan.target_root, workspace_root.join(".trae"));
        assert_eq!(plan.commit, fixture.second);
        assert_eq!(plan.status, DistributionPlanStatus::Ready);
    }

    #[test]
    fn distribution_plans_and_execution_match_cli_for_prompts_and_skills() {
        let fixture = VersionFixture::new();
        write_renderable_prompt(&fixture.root, "prod prompt");
        commit_all(&fixture.root, "renderable prompt");
        let prod_commit = head(&fixture.root);
        create_prod(&fixture, "prompt:review", &prod_commit);
        create_prod(&fixture, "skill:onboard", &prod_commit);

        let options = get_distribution_options(fixture.path(), "prompt:review".to_owned()).unwrap();
        assert_eq!(options.default_revision, "adock/prompt/review/prod");
        assert_eq!(
            options.supported_targets,
            ["claude-code", "cursor", "trae", "workbuddy"]
        );
        assert!(options.targets.iter().any(|target| target.target == "trae"));

        let variables = BTreeMap::from([
            ("change".to_owned(), "PATCH".to_owned()),
            ("tone".to_owned(), "detailed".to_owned()),
        ]);
        let prompt_plan = preview_distribution(
            fixture.path(),
            "prompt:review".to_owned(),
            "trae".to_owned(),
            None,
            variables.clone(),
            false,
        )
        .unwrap();
        assert_eq!(prompt_plan.status, DistributionPlanStatus::Ready);
        assert_eq!(prompt_plan.commit, prod_commit);
        assert!(prompt_plan.files.iter().any(|file| {
            String::from_utf8_lossy(&file.content)
                .contains("Review PATCH in a detailed style.\nprod prompt")
        }));
        let output = fixture
            .cli()
            .args([
                "distribute",
                "prompt",
                "review",
                "--target",
                "trae",
                "--var",
                "change=PATCH",
                "--var",
                "tone=detailed",
                "--dry-run",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        assert_cli_matches_plan(&output, &prompt_plan);

        let error = execute_distribution(prompt_plan.clone(), false).unwrap_err();
        assert_eq!(error.code, AppErrorCode::ConfirmationRequired);
        assert!(
            prompt_plan
                .files
                .iter()
                .all(|file| !file.target_path.exists())
        );
        let execution = execute_distribution(prompt_plan.clone(), true).unwrap();
        assert_eq!(execution.status, DistributionExecutionStatus::Completed);
        for file in &prompt_plan.files {
            assert_eq!(fs::read(&file.target_path).unwrap(), file.content);
        }
        let output = fixture
            .cli()
            .args([
                "distribute",
                "prompt",
                "review",
                "--target",
                "trae",
                "--var",
                "change=PATCH",
                "--var",
                "tone=detailed",
                "--yes",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("Execution status: completed"));
        assert!(output.contains("unchanged |"));

        write_renderable_prompt(&fixture.root, "head prompt");
        commit_all(&fixture.root, "head-only prompt");
        let head_commit = head(&fixture.root);
        let head_plan = preview_distribution(
            fixture.path(),
            "prompt:review".to_owned(),
            "trae".to_owned(),
            Some("HEAD".to_owned()),
            variables.clone(),
            false,
        )
        .unwrap();
        assert_eq!(head_plan.commit, head_commit);
        assert!(
            head_plan
                .files
                .iter()
                .any(|file| { String::from_utf8_lossy(&file.content).contains("head prompt") })
        );
        let default_plan = preview_distribution(
            fixture.path(),
            "prompt:review".to_owned(),
            "trae".to_owned(),
            None,
            variables,
            false,
        )
        .unwrap();
        assert_eq!(default_plan.commit, prod_commit);
        assert!(
            default_plan
                .files
                .iter()
                .all(|file| { !String::from_utf8_lossy(&file.content).contains("head prompt") })
        );
        let output = fixture
            .cli()
            .args([
                "distribute",
                "prompt",
                "review",
                "--target",
                "trae",
                "--ref",
                "HEAD",
                "--var",
                "change=PATCH",
                "--var",
                "tone=detailed",
                "--dry-run",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        assert_cli_matches_plan(&output, &head_plan);

        let skill_options =
            get_distribution_options(fixture.path(), "skill:onboard".to_owned()).unwrap();
        assert_eq!(skill_options.default_revision, "adock/skill/onboard/prod");
        assert!(
            skill_options
                .targets
                .iter()
                .any(|target| target.target == "codex")
        );
        let skill_plan = preview_distribution(
            fixture.path(),
            "skill:onboard".to_owned(),
            "codex".to_owned(),
            None,
            BTreeMap::new(),
            false,
        )
        .unwrap();
        assert_eq!(skill_plan.status, DistributionPlanStatus::Ready);
        assert_eq!(skill_plan.commit, prod_commit);
        assert!(skill_plan.files.len() >= 2);
        let output = fixture
            .cli()
            .args([
                "distribute",
                "skill",
                "onboard",
                "--target",
                "codex",
                "--dry-run",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        assert_cli_matches_plan(&output, &skill_plan);
        let error = execute_distribution(skill_plan.clone(), false).unwrap_err();
        assert_eq!(error.code, AppErrorCode::ConfirmationRequired);
        assert!(
            skill_plan
                .files
                .iter()
                .all(|file| !file.target_path.exists())
        );
        let execution = execute_distribution(skill_plan.clone(), true).unwrap();
        assert_eq!(execution.status, DistributionExecutionStatus::Completed);
        for file in &skill_plan.files {
            assert_eq!(fs::read(&file.target_path).unwrap(), file.content);
        }
    }

    #[test]
    fn distribution_conflicts_match_cli_and_require_managed_only_overwrite() {
        let fixture = VersionFixture::new();
        create_prod(&fixture, "prompt:review", &fixture.second);
        let initial = preview_distribution(
            fixture.path(),
            "prompt:review".to_owned(),
            "trae".to_owned(),
            None,
            BTreeMap::new(),
            false,
        )
        .unwrap();
        let target_path = initial.files[0].target_path.clone();
        fs::create_dir_all(target_path.parent().unwrap()).unwrap();
        fs::write(&target_path, "unmanaged\n").unwrap();

        let blocked = preview_distribution(
            fixture.path(),
            "prompt:review".to_owned(),
            "trae".to_owned(),
            None,
            BTreeMap::new(),
            true,
        )
        .unwrap();
        assert_eq!(blocked.status, DistributionPlanStatus::Blocked);
        assert_eq!(blocked.files[0].status, PlannedFileStatus::Conflict);
        assert_eq!(
            blocked.files[0].conflict,
            Some(DistributionConflict::UnmanagedFile)
        );
        let assert = fixture
            .cli()
            .args([
                "distribute",
                "prompt",
                "review",
                "--target",
                "trae",
                "--overwrite-managed",
                "--dry-run",
            ])
            .assert()
            .failure();
        let output = assert.get_output();
        assert_cli_matches_plan(&output.stdout, &blocked);
        assert!(String::from_utf8_lossy(&output.stderr).contains("error[distribution_conflict]"));
        let mut forged = blocked.clone();
        forged.status = DistributionPlanStatus::Ready;
        forged.overwrite_managed = true;
        forged.files[0].status = PlannedFileStatus::Updated;
        forged.files[0].conflict = None;
        let error = execute_distribution(forged, true).unwrap_err();
        assert_eq!(error.code, AppErrorCode::DistributionConflict);
        let error = execute_distribution(blocked, true).unwrap_err();
        assert_eq!(error.code, AppErrorCode::DistributionConflict);
        assert_eq!(fs::read_to_string(&target_path).unwrap(), "unmanaged\n");

        fs::remove_file(&target_path).unwrap();
        let ready = preview_distribution(
            fixture.path(),
            "prompt:review".to_owned(),
            "trae".to_owned(),
            None,
            BTreeMap::new(),
            false,
        )
        .unwrap();
        execute_distribution(ready, true).unwrap();
        fs::write(&target_path, "local edit\n").unwrap();

        let managed_blocked = preview_distribution(
            fixture.path(),
            "prompt:review".to_owned(),
            "trae".to_owned(),
            None,
            BTreeMap::new(),
            false,
        )
        .unwrap();
        assert_eq!(managed_blocked.status, DistributionPlanStatus::Blocked);
        assert_eq!(
            managed_blocked.files[0].conflict,
            Some(DistributionConflict::LocallyModified)
        );
        let output = fixture
            .cli()
            .args([
                "distribute",
                "prompt",
                "review",
                "--target",
                "trae",
                "--dry-run",
            ])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        assert_cli_matches_plan(&output, &managed_blocked);

        let overwrite = preview_distribution(
            fixture.path(),
            "prompt:review".to_owned(),
            "trae".to_owned(),
            None,
            BTreeMap::new(),
            true,
        )
        .unwrap();
        assert_eq!(overwrite.status, DistributionPlanStatus::Ready);
        assert_eq!(overwrite.files[0].status, PlannedFileStatus::Updated);
        let output = fixture
            .cli()
            .args([
                "distribute",
                "prompt",
                "review",
                "--target",
                "trae",
                "--overwrite-managed",
                "--dry-run",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        assert_cli_matches_plan(&output, &overwrite);
        let error = execute_distribution(overwrite.clone(), false).unwrap_err();
        assert_eq!(error.code, AppErrorCode::ConfirmationRequired);
        assert_eq!(fs::read_to_string(&target_path).unwrap(), "local edit\n");
        let execution = execute_distribution(overwrite.clone(), true).unwrap();
        assert_eq!(execution.status, DistributionExecutionStatus::Completed);
        assert_eq!(fs::read(&target_path).unwrap(), overwrite.files[0].content);
    }
}
