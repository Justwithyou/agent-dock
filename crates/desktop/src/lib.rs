use std::path::Path;

use adock_adapters::{GitLogEntry, GitRepository};
use adock_core::{
    AppError, Asset, AssetDiff, AssetRef, RevisionSelector, ValidationReport, VersionTag,
    WorkspaceInfo, WorkspaceService,
};
use adock_infra::{
    AssetFilter, AssetService, FileSystemWorkspaceDiscovery, WorkspaceInitializer,
    load_workspace_config,
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
fn list_assets(path: String) -> Result<Vec<DesktopAssetListItem>, AppError> {
    let service = AssetService::new(path);
    let result = service.list(&AssetFilter::default())?;
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
            rollback_asset
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

    use adock_core::AppErrorCode;
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
                "default_channel: prod\ntargets: {}\n",
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

    #[test]
    fn asset_commands_share_core_services_and_isolate_broken_assets() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("prompts")).unwrap();
        fs::write(
            temp.path().join("prompts/review.md"),
            "---\napiVersion: adock/v1alpha1\nkind: prompt\nid: review\ntitle: Review\ndescription: Review code\nowners: [alice]\nspec:\n  variables: []\n---\nReview code\n",
        )
        .unwrap();
        fs::write(temp.path().join("prompts/broken.md"), "broken").unwrap();

        let entries = list_assets(temp.path().display().to_string()).unwrap();
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
}
