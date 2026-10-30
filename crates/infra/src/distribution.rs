use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

use adock_core::{
    AdapterOutput, AppError, AppErrorCode, AssetRef, AssetSnapshot, DistributionAdapter,
    DistributionConflict, DistributionExecution, DistributionExecutionStatus,
    DistributionFileResult, DistributionFileStatus, DistributionPlan, DistributionPlanStatus,
    PlannedFile, PlannedFileStatus, RevisionSelector, TargetProfile,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

const MANIFEST_VERSION: u32 = 1;
const MANIFEST_DIRECTORY: &str = ".adock";
const MANIFEST_FILE: &str = "distribution-manifest.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionManifestEntry {
    pub asset: AssetRef,
    pub commit: String,
    pub adapter_version: String,
    pub path: String,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionManifest {
    pub version: u32,
    pub files: BTreeMap<String, DistributionManifestEntry>,
}

impl Default for DistributionManifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            files: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DistributionService;

impl DistributionService {
    pub fn create_plan(
        snapshot: &AssetSnapshot,
        target: &TargetProfile,
        adapter: &dyn DistributionAdapter,
        overwrite_managed: bool,
    ) -> Result<DistributionPlan, AppError> {
        let asset = snapshot.asset.reference()?;
        let commit = require_commit(&snapshot.revision, snapshot.commit.as_deref())?;
        let AdapterOutput {
            adapter_version,
            mut files,
        } = adapter.adapt(snapshot)?;
        validate_adapter_version(&adapter_version)?;
        if files.is_empty() {
            return Err(AppError::new(
                AppErrorCode::InvalidAsset,
                "distribution.plan",
                asset.to_string(),
                "the adapter produced no files",
                "Use an adapter that emits at least one target file",
            ));
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));

        let manifest = load_manifest(&target.root)?;
        let mut seen = BTreeSet::new();
        let mut planned_files = Vec::with_capacity(files.len());
        for file in files {
            let target_path = portable_target_path(&target.root, &file.path)?;
            if !seen.insert(file.path.clone()) {
                return Err(invalid_path(
                    &file.path,
                    "the adapter emitted a duplicate path",
                ));
            }
            ensure_safe_components(&target.root, &file.path)?;
            let managed = manifest.files.get(&file.path);
            let state = target_state(&target_path)?;
            let checksum = sha256(&file.content);
            let previous_checksum = match state {
                TargetState::File(ref checksum) => Some(checksum.clone()),
                TargetState::Missing | TargetState::Other => None,
            };
            let (status, conflict) = match state {
                TargetState::Missing => (PlannedFileStatus::Added, None),
                TargetState::Other => (
                    PlannedFileStatus::Conflict,
                    Some(DistributionConflict::UnmanagedFile),
                ),
                TargetState::File(actual_checksum) => match managed {
                    None => (
                        PlannedFileStatus::Conflict,
                        Some(DistributionConflict::UnmanagedFile),
                    ),
                    Some(entry) if entry.checksum == actual_checksum => {
                        if actual_checksum == checksum {
                            (PlannedFileStatus::Unchanged, None)
                        } else {
                            (PlannedFileStatus::Updated, None)
                        }
                    }
                    Some(_) if overwrite_managed => (PlannedFileStatus::Updated, None),
                    Some(_) => (
                        PlannedFileStatus::Conflict,
                        Some(DistributionConflict::LocallyModified),
                    ),
                },
            };
            planned_files.push(PlannedFile {
                target_path,
                relative_path: file.path,
                checksum,
                content: file.content,
                status,
                conflict,
                previous_checksum,
                managed_checksum: managed.map(|entry| entry.checksum.clone()),
            });
        }
        let status = if planned_files.iter().any(|file| file.conflict.is_some()) {
            DistributionPlanStatus::Blocked
        } else {
            DistributionPlanStatus::Ready
        };
        Ok(DistributionPlan {
            asset,
            target: target.target.clone(),
            target_root: target.root.clone(),
            source: snapshot.revision.clone(),
            commit,
            adapter_version,
            status,
            files: planned_files,
        })
    }

    pub fn execute(plan: &DistributionPlan) -> Result<DistributionExecution, AppError> {
        Self::execute_with_writer(plan, |path, content| {
            atomic_write(path, content, "distribution.file.write")
        })
    }

    fn execute_with_writer<F>(
        plan: &DistributionPlan,
        mut writer: F,
    ) -> Result<DistributionExecution, AppError>
    where
        F: FnMut(&Path, &[u8]) -> Result<(), AppError>,
    {
        let mut manifest = preflight(plan)?;
        let mut results = Vec::with_capacity(plan.files.len());
        for (index, file) in plan.files.iter().enumerate() {
            let entry = DistributionManifestEntry {
                asset: plan.asset.clone(),
                commit: plan.commit.clone(),
                adapter_version: plan.adapter_version.clone(),
                path: file.relative_path.clone(),
                checksum: file.checksum.clone(),
            };
            let outcome = match file.status {
                PlannedFileStatus::Unchanged => {
                    update_manifest(&plan.target_root, &mut manifest, entry)
                        .map(|()| DistributionFileStatus::Unchanged)
                }
                PlannedFileStatus::Added | PlannedFileStatus::Updated => {
                    writer(&file.target_path, &file.content)
                        .and_then(|()| update_manifest(&plan.target_root, &mut manifest, entry))
                        .map(|()| DistributionFileStatus::Written)
                }
                PlannedFileStatus::Conflict => Err(conflict_error(
                    &file.relative_path,
                    "a conflict remained in an executable plan",
                )),
            };
            match outcome {
                Ok(status) => results.push(DistributionFileResult {
                    target_path: file.target_path.clone(),
                    relative_path: file.relative_path.clone(),
                    status,
                    error: None,
                }),
                Err(error) => {
                    results.push(DistributionFileResult {
                        target_path: file.target_path.clone(),
                        relative_path: file.relative_path.clone(),
                        status: DistributionFileStatus::Failed,
                        error: Some(error),
                    });
                    results.extend(plan.files[index + 1..].iter().map(|skipped| {
                        DistributionFileResult {
                            target_path: skipped.target_path.clone(),
                            relative_path: skipped.relative_path.clone(),
                            status: DistributionFileStatus::Skipped,
                            error: None,
                        }
                    }));
                    break;
                }
            }
        }
        let failed = results
            .iter()
            .any(|result| result.status == DistributionFileStatus::Failed);
        let processed = results.iter().any(|result| {
            matches!(
                result.status,
                DistributionFileStatus::Written | DistributionFileStatus::Unchanged
            )
        });
        let status = match (failed, processed) {
            (false, _) => DistributionExecutionStatus::Completed,
            (true, true) => DistributionExecutionStatus::PartialFailed,
            (true, false) => DistributionExecutionStatus::Failed,
        };
        Ok(DistributionExecution {
            status,
            files: results,
        })
    }
}

fn require_commit(source: &RevisionSelector, commit: Option<&str>) -> Result<String, AppError> {
    if matches!(source, RevisionSelector::WorkingTree) {
        return Err(AppError::new(
            AppErrorCode::InvalidRevision,
            "distribution.source",
            source.to_string(),
            "working-tree snapshots cannot be formally distributed",
            "Commit the asset and choose HEAD, a commit SHA, or an asset tag",
        ));
    }
    let commit = commit.ok_or_else(|| {
        AppError::new(
            AppErrorCode::InvalidRevision,
            "distribution.source",
            source.to_string(),
            "the distribution source has no traceable commit SHA",
            "Choose a committed asset snapshot",
        )
    })?;
    match RevisionSelector::parse(commit) {
        Ok(RevisionSelector::Commit(_)) => {}
        _ => {
            return Err(AppError::new(
                AppErrorCode::InvalidRevision,
                "distribution.source",
                commit,
                "the snapshot commit is not a full commit SHA",
                "Read a fresh committed snapshot before planning distribution",
            ));
        }
    }
    if matches!(source, RevisionSelector::Commit(source_sha) if source_sha != commit) {
        return Err(AppError::new(
            AppErrorCode::InvalidRevision,
            "distribution.source",
            source.to_string(),
            "the snapshot commit does not match its revision selector",
            "Read a consistent committed snapshot before planning distribution",
        ));
    }
    Ok(commit.to_owned())
}

fn validate_adapter_version(version: &str) -> Result<(), AppError> {
    if version.trim().is_empty() || version.trim() != version {
        return Err(AppError::new(
            AppErrorCode::InvalidInput,
            "distribution.adapter",
            version,
            "adapter version must be a non-empty stable identifier",
            "Set an adapter version such as claude-code-skill/v1",
        ));
    }
    Ok(())
}

fn portable_target_path(root: &Path, relative: &str) -> Result<PathBuf, AppError> {
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.contains(['\\', ':', '\0'])
        || Path::new(relative).is_absolute()
        || Path::new(relative)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid_path(
            relative,
            "expected a non-empty portable relative path using forward slashes",
        ));
    }
    let components = relative.split('/').collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| component.is_empty() || matches!(*component, "." | ".."))
    {
        return Err(invalid_path(
            relative,
            "path components cannot be empty, '.' or '..'",
        ));
    }
    if components[0].eq_ignore_ascii_case(MANIFEST_DIRECTORY) {
        return Err(invalid_path(
            relative,
            "the .adock directory is reserved for distribution manifests",
        ));
    }
    Ok(components
        .iter()
        .fold(root.to_path_buf(), |path, component| path.join(component)))
}

fn invalid_path(path: &str, reason: &str) -> AppError {
    AppError::new(
        AppErrorCode::InvalidInput,
        "distribution.path",
        path,
        reason,
        "Use a portable relative path inside the configured target root",
    )
}

fn ensure_root_safe(root: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(invalid_path(
            &root.display().to_string(),
            "the target root cannot be a symbolic link",
        )),
        Ok(metadata) if !metadata.is_dir() => Err(invalid_path(
            &root.display().to_string(),
            "the target root is not a directory",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(target_read_error(root, error)),
    }
}

fn ensure_safe_components(root: &Path, relative: &str) -> Result<(), AppError> {
    ensure_root_safe(root)?;
    let mut current = root.to_path_buf();
    let components = relative.split('/').collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(invalid_path(
                    &current.display().to_string(),
                    "symbolic-link path components cannot be used for distribution",
                ));
            }
            Ok(metadata) if index + 1 < components.len() && !metadata.is_dir() => {
                return Err(invalid_path(
                    &current.display().to_string(),
                    "a parent path component is not a directory",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(target_read_error(&current, error)),
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TargetState {
    Missing,
    File(String),
    Other,
}

fn target_state(path: &Path) -> Result<TargetState, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            let content = fs::read(path).map_err(|error| target_read_error(path, error))?;
            Ok(TargetState::File(sha256(&content)))
        }
        Ok(_) => Ok(TargetState::Other),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(TargetState::Missing),
        Err(error) => Err(target_read_error(path, error)),
    }
}

fn target_read_error(path: &Path, error: std::io::Error) -> AppError {
    AppError::new(
        AppErrorCode::WorkspaceIo,
        "distribution.target.read",
        path.display().to_string(),
        error.to_string(),
        "Check the target path and its permissions, then regenerate the plan",
    )
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join(MANIFEST_DIRECTORY).join(MANIFEST_FILE)
}

fn load_manifest(root: &Path) -> Result<DistributionManifest, AppError> {
    ensure_root_safe(root)?;
    let directory = root.join(MANIFEST_DIRECTORY);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(invalid_path(
                &directory.display().to_string(),
                "the manifest directory must be a real directory, not a file or symbolic link",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DistributionManifest::default());
        }
        Err(error) => return Err(target_read_error(&directory, error)),
    }
    let path = manifest_path(root);
    let input = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            fs::read(&path).map_err(|error| target_read_error(&path, error))?
        }
        Ok(_) => {
            return Err(invalid_path(
                &path.display().to_string(),
                "the distribution manifest must be a regular file",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DistributionManifest::default());
        }
        Err(error) => return Err(target_read_error(&path, error)),
    };
    let manifest: DistributionManifest = serde_json::from_slice(&input).map_err(|error| {
        AppError::new(
            AppErrorCode::InvalidConfig,
            "distribution.manifest.read",
            path.display().to_string(),
            format!("invalid distribution manifest: {error}"),
            "Repair the manifest or move it aside only after preserving unmanaged target files",
        )
    })?;
    if manifest.version != MANIFEST_VERSION
        || manifest
            .files
            .iter()
            .any(|(path, entry)| path != &entry.path)
    {
        return Err(AppError::new(
            AppErrorCode::InvalidConfig,
            "distribution.manifest.read",
            path.display().to_string(),
            "unsupported manifest version or inconsistent file entries",
            "Restore a valid AgentDock distribution manifest",
        ));
    }
    Ok(manifest)
}

fn preflight(plan: &DistributionPlan) -> Result<DistributionManifest, AppError> {
    TargetProfile::new(plan.target.clone(), plan.target_root.clone())?;
    require_commit(&plan.source, Some(&plan.commit))?;
    validate_adapter_version(&plan.adapter_version)?;
    if plan.status == DistributionPlanStatus::Blocked {
        let file = plan
            .files
            .iter()
            .find(|file| file.conflict.is_some())
            .ok_or_else(|| {
                conflict_error(&plan.target, "the blocked plan has no conflict detail")
            })?;
        let reason = match file.conflict {
            Some(DistributionConflict::UnmanagedFile) => {
                "an unmanaged target file or directory blocks distribution"
            }
            Some(DistributionConflict::LocallyModified) => {
                "a managed target file has local modifications"
            }
            None => "the plan is blocked",
        };
        return Err(conflict_error(&file.relative_path, reason));
    }
    if plan.files.is_empty() {
        return Err(AppError::new(
            AppErrorCode::InvalidInput,
            "distribution.execute",
            plan.asset.to_string(),
            "cannot execute an empty distribution plan",
            "Regenerate a plan with at least one adapter file",
        ));
    }
    let manifest = load_manifest(&plan.target_root)?;
    let mut seen = BTreeSet::new();
    for file in &plan.files {
        let expected_path = portable_target_path(&plan.target_root, &file.relative_path)?;
        if expected_path != file.target_path {
            return Err(conflict_error(
                &file.relative_path,
                "the planned absolute path does not match its target root",
            ));
        }
        if !seen.insert(&file.relative_path) || sha256(&file.content) != file.checksum {
            return Err(conflict_error(
                &file.relative_path,
                "the plan contains duplicate paths or inconsistent content checksums",
            ));
        }
        ensure_safe_components(&plan.target_root, &file.relative_path)?;
        let current = match target_state(&file.target_path)? {
            TargetState::Missing => None,
            TargetState::File(checksum) => Some(checksum),
            TargetState::Other => {
                return Err(conflict_error(
                    &file.relative_path,
                    "the target changed to a non-file after planning; regenerate the plan",
                ));
            }
        };
        let managed = manifest
            .files
            .get(&file.relative_path)
            .map(|entry| entry.checksum.clone());
        if current != file.previous_checksum || managed != file.managed_checksum {
            return Err(conflict_error(
                &file.relative_path,
                "the target file or manifest changed after planning; regenerate the plan",
            ));
        }
    }
    Ok(manifest)
}

fn conflict_error(subject: &str, reason: &str) -> AppError {
    AppError::new(
        AppErrorCode::DistributionConflict,
        "distribution.conflict",
        subject,
        reason,
        "Review the target state and regenerate the plan; unmanaged files are never overwritten",
    )
}

fn update_manifest(
    root: &Path,
    manifest: &mut DistributionManifest,
    entry: DistributionManifestEntry,
) -> Result<(), AppError> {
    if manifest.files.get(&entry.path) == Some(&entry) {
        return Ok(());
    }
    let mut updated = manifest.clone();
    updated.files.insert(entry.path.clone(), entry);
    save_manifest(root, &updated)?;
    *manifest = updated;
    Ok(())
}

fn save_manifest(root: &Path, manifest: &DistributionManifest) -> Result<(), AppError> {
    let directory = root.join(MANIFEST_DIRECTORY);
    fs::create_dir_all(&directory)
        .map_err(|error| file_write_error("distribution.manifest.write", &directory, error))?;
    let mut content = serde_json::to_vec_pretty(manifest).map_err(|error| {
        AppError::new(
            AppErrorCode::FileWrite,
            "distribution.manifest.write",
            manifest_path(root).display().to_string(),
            error.to_string(),
            "Check the manifest data and retry",
        )
    })?;
    content.push(b'\n');
    atomic_write(
        &manifest_path(root),
        &content,
        "distribution.manifest.write",
    )
}

fn atomic_write(path: &Path, content: &[u8], operation: &'static str) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| {
        AppError::new(
            AppErrorCode::FileWrite,
            operation,
            path.display().to_string(),
            "the target path has no parent directory",
            "Regenerate the distribution plan with a valid target path",
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| file_write_error(operation, parent, error))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| file_write_error(operation, parent, error))?;
    temporary
        .write_all(content)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| file_write_error(operation, path, error))?;
    temporary
        .persist(path)
        .map_err(|error| file_write_error(operation, path, error.error))?;
    Ok(())
}

fn file_write_error(operation: &'static str, path: &Path, error: std::io::Error) -> AppError {
    AppError::new(
        AppErrorCode::FileWrite,
        operation,
        path.display().to_string(),
        error.to_string(),
        "Check file permissions and available space, then regenerate the plan and retry",
    )
}

fn sha256(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use adock_core::{AdapterFile, parse_asset};
    use tempfile::TempDir;

    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    struct StaticAdapter {
        files: Vec<AdapterFile>,
    }

    impl DistributionAdapter for StaticAdapter {
        fn adapt(&self, _snapshot: &AssetSnapshot) -> Result<AdapterOutput, AppError> {
            Ok(AdapterOutput {
                adapter_version: "test-prompt/v1".to_owned(),
                files: self.files.clone(),
            })
        }
    }

    fn snapshot() -> AssetSnapshot {
        let asset = parse_asset(
            Path::new("prompts/review.md"),
            "---\napiVersion: adock/v1alpha1\nkind: prompt\nid: review\ntitle: Review\ndescription: Review code\nowners: [alice]\nspec:\n  variables: []\n---\nReview code\n",
        )
        .unwrap();
        AssetSnapshot {
            asset,
            revision: RevisionSelector::Head,
            commit: Some(COMMIT.to_owned()),
            files: Vec::new(),
        }
    }

    fn adapter(paths: &[&str]) -> StaticAdapter {
        StaticAdapter {
            files: paths
                .iter()
                .map(|path| AdapterFile {
                    path: (*path).to_owned(),
                    content: format!("content for {path}\n").into_bytes(),
                })
                .collect(),
        }
    }

    fn target(temp: &TempDir) -> TargetProfile {
        TargetProfile::new("test", temp.path().join("target")).unwrap()
    }

    #[test]
    fn rejects_non_portable_paths_and_working_tree_sources_before_writing() {
        let temp = TempDir::new().unwrap();
        for path in [
            "/absolute",
            "C:/drive",
            "../escape",
            "skills//empty",
            "skills/./dot",
            "skills\\windows",
            ".adock/distribution-manifest.json",
        ] {
            let error = DistributionService::create_plan(
                &snapshot(),
                &target(&temp),
                &adapter(&[path]),
                false,
            )
            .unwrap_err();
            assert_eq!(error.code, AppErrorCode::InvalidInput, "path: {path}");
        }

        let mut working = snapshot();
        working.revision = RevisionSelector::WorkingTree;
        let error = DistributionService::create_plan(
            &working,
            &target(&temp),
            &adapter(&["skills/review/SKILL.md"]),
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidRevision);
        assert!(!temp.path().join("target").exists());
    }

    #[test]
    fn unmanaged_files_always_block_and_managed_local_changes_require_explicit_overwrite() {
        let temp = TempDir::new().unwrap();
        let target = target(&temp);
        let adapter = adapter(&["skills/review/SKILL.md"]);
        let path = target.root.join("skills/review/SKILL.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "unmanaged").unwrap();

        let blocked =
            DistributionService::create_plan(&snapshot(), &target, &adapter, true).unwrap();
        assert_eq!(blocked.status, DistributionPlanStatus::Blocked);
        assert_eq!(
            blocked.files[0].conflict,
            Some(DistributionConflict::UnmanagedFile)
        );
        assert_eq!(
            DistributionService::execute(&blocked).unwrap_err().code,
            AppErrorCode::DistributionConflict
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "unmanaged");

        fs::remove_file(&path).unwrap();
        let initial =
            DistributionService::create_plan(&snapshot(), &target, &adapter, false).unwrap();
        assert_eq!(
            DistributionService::execute(&initial).unwrap().status,
            DistributionExecutionStatus::Completed
        );
        fs::write(&path, "local edit").unwrap();

        let blocked =
            DistributionService::create_plan(&snapshot(), &target, &adapter, false).unwrap();
        assert_eq!(
            blocked.files[0].conflict,
            Some(DistributionConflict::LocallyModified)
        );
        let overwrite =
            DistributionService::create_plan(&snapshot(), &target, &adapter, true).unwrap();
        assert_eq!(overwrite.status, DistributionPlanStatus::Ready);
        assert_eq!(overwrite.files[0].status, PlannedFileStatus::Updated);
        DistributionService::execute(&overwrite).unwrap();
        assert_eq!(fs::read(&path).unwrap(), adapter.files[0].content);
    }

    #[test]
    fn executes_the_same_plan_idempotently_and_records_commit_traceability() {
        let temp = TempDir::new().unwrap();
        let target = target(&temp);
        let adapter = adapter(&["skills/review/SKILL.md"]);
        let plan = DistributionService::create_plan(&snapshot(), &target, &adapter, false).unwrap();
        let expected_files = plan.files.clone();
        let first = DistributionService::execute(&plan).unwrap();
        assert_eq!(first.status, DistributionExecutionStatus::Completed);
        assert_eq!(
            plan.files, expected_files,
            "execution must not mutate the preview plan"
        );
        let manifest_before = fs::read(manifest_path(&target.root)).unwrap();
        let manifest = load_manifest(&target.root).unwrap();
        let entry = &manifest.files["skills/review/SKILL.md"];
        assert_eq!(entry.commit, COMMIT);
        assert_eq!(entry.asset.to_string(), "prompt:review");
        assert_eq!(entry.checksum, sha256(&adapter.files[0].content));

        let repeat =
            DistributionService::create_plan(&snapshot(), &target, &adapter, false).unwrap();
        assert_eq!(repeat.files[0].status, PlannedFileStatus::Unchanged);
        let writes = AtomicUsize::new(0);
        let second = DistributionService::execute_with_writer(&repeat, |_, _| {
            writes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .unwrap();
        assert_eq!(second.status, DistributionExecutionStatus::Completed);
        assert_eq!(writes.load(Ordering::Relaxed), 0);
        assert_eq!(
            fs::read(manifest_path(&target.root)).unwrap(),
            manifest_before,
            "identical content and commit must leave the manifest untouched"
        );
    }

    #[test]
    fn partial_failure_is_explicit_and_a_fresh_plan_retries_remaining_files() {
        let temp = TempDir::new().unwrap();
        let target = target(&temp);
        let adapter = adapter(&[
            "commands/first.md",
            "commands/second.md",
            "commands/third.md",
        ]);
        let plan = DistributionService::create_plan(&snapshot(), &target, &adapter, false).unwrap();
        let result = DistributionService::execute_with_writer(&plan, |path, content| {
            if path.ends_with("second.md") {
                Err(file_write_error(
                    "distribution.file.write",
                    path,
                    std::io::Error::new(std::io::ErrorKind::PermissionDenied, "injected failure"),
                ))
            } else {
                atomic_write(path, content, "distribution.file.write")
            }
        })
        .unwrap();

        assert_eq!(result.status, DistributionExecutionStatus::PartialFailed);
        assert_eq!(
            result
                .files
                .iter()
                .map(|file| file.status)
                .collect::<Vec<_>>(),
            vec![
                DistributionFileStatus::Written,
                DistributionFileStatus::Failed,
                DistributionFileStatus::Skipped,
            ]
        );
        let manifest = load_manifest(&target.root).unwrap();
        assert!(manifest.files.contains_key("commands/first.md"));
        assert!(!manifest.files.contains_key("commands/second.md"));

        let retry =
            DistributionService::create_plan(&snapshot(), &target, &adapter, false).unwrap();
        assert_eq!(retry.files[0].status, PlannedFileStatus::Unchanged);
        assert_eq!(retry.files[1].status, PlannedFileStatus::Added);
        assert_eq!(retry.files[2].status, PlannedFileStatus::Added);
        let completed = DistributionService::execute(&retry).unwrap();
        assert_eq!(completed.status, DistributionExecutionStatus::Completed);
        assert_eq!(load_manifest(&target.root).unwrap().files.len(), 3);
    }

    #[test]
    fn stale_target_state_blocks_execution_before_any_write() {
        let temp = TempDir::new().unwrap();
        let target = target(&temp);
        let adapter = adapter(&["commands/first.md", "commands/second.md"]);
        let plan = DistributionService::create_plan(&snapshot(), &target, &adapter, false).unwrap();
        fs::create_dir_all(target.root.join("commands")).unwrap();
        fs::write(target.root.join("commands/second.md"), "racing change").unwrap();

        let error = DistributionService::execute(&plan).unwrap_err();
        assert_eq!(error.code, AppErrorCode::DistributionConflict);
        assert!(!target.root.join("commands/first.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn existing_symbolic_link_components_are_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let target = target(&temp);
        fs::create_dir_all(temp.path().join("real")).unwrap();
        symlink(temp.path().join("real"), target.root.join("skills")).unwrap();
        let error = DistributionService::create_plan(
            &snapshot(),
            &target,
            &adapter(&["skills/review/SKILL.md"]),
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidInput);
    }
}
