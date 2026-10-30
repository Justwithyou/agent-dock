use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use adock_core::{
    AppError, AppErrorCode, AssetDiff, AssetRef, AssetSnapshot, ChangeKind, RevisionSelector,
    SnapshotFile, VersionTag, diff_snapshots, parse_asset_with_sources,
};
use adock_infra::AssetService;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct GitCli {
    executable: PathBuf,
}

impl Default for GitCli {
    fn default() -> Self {
        Self::new()
    }
}

impl GitCli {
    pub fn new() -> Self {
        Self {
            executable: PathBuf::from("git"),
        }
    }

    pub fn with_executable(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    fn run(&self, root: &Path, args: &[&OsStr]) -> Result<Output, AppError> {
        Command::new(&self.executable)
            .args(args)
            .current_dir(root)
            .output()
            .map_err(|error| {
                let code = if error.kind() == std::io::ErrorKind::NotFound {
                    AppErrorCode::GitUnavailable
                } else {
                    AppErrorCode::GitCommandFailed
                };
                AppError::new(
                    code,
                    "git.execute",
                    self.executable.display().to_string(),
                    error.to_string(),
                    "Install Git and ensure it is available on PATH",
                )
            })
    }

    fn output(&self, root: &Path, args: &[&OsStr], operation: &str) -> Result<Vec<u8>, AppError> {
        let output = self.run(root, args)?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(command_failed(operation, root, &output))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRevision {
    pub selector: RevisionSelector,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitLogEntry {
    pub commit: String,
    pub author: String,
    pub committed_at: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct GitRepository {
    root: PathBuf,
    git: GitCli,
}

impl GitRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        Self::open_with_git(path, GitCli::new())
    }

    pub fn open_with_git(path: impl AsRef<Path>, git: GitCli) -> Result<Self, AppError> {
        let path = path.as_ref().canonicalize().map_err(|error| {
            AppError::new(
                AppErrorCode::WorkspaceIo,
                "git.open",
                path.as_ref().display().to_string(),
                error.to_string(),
                "Choose an existing workspace directory",
            )
        })?;
        let output = git.run(
            &path,
            &[OsStr::new("rev-parse"), OsStr::new("--show-toplevel")],
        )?;
        if !output.status.success() {
            return Err(AppError::new(
                AppErrorCode::NotGitRepository,
                "git.open",
                path.display().to_string(),
                "the workspace is not inside a Git working tree",
                "Initialize or clone a Git repository before using version operations",
            ));
        }
        let root = utf8(&output.stdout, "git.open", path.display().to_string())?
            .trim()
            .to_owned();
        let root = PathBuf::from(root).canonicalize().map_err(|error| {
            AppError::new(
                AppErrorCode::NotGitRepository,
                "git.open",
                path.display().to_string(),
                error.to_string(),
                "Choose a valid Git working tree",
            )
        })?;
        Ok(Self { root, git })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn resolve(&self, selector: &RevisionSelector) -> Result<ResolvedRevision, AppError> {
        validate_selector(selector)?;
        let commit = match selector {
            RevisionSelector::WorkingTree => match self.commit_at(&RevisionSelector::Head) {
                Ok(commit) => Some(commit),
                Err(error) if error.code == AppErrorCode::UnknownRevision => None,
                Err(error) => return Err(error),
            },
            _ => Some(self.commit_at(selector)?),
        };
        Ok(ResolvedRevision {
            selector: selector.clone(),
            commit,
        })
    }

    pub fn snapshot(
        &self,
        reference: &AssetRef,
        selector: &RevisionSelector,
    ) -> Result<AssetSnapshot, AppError> {
        reference.ensure_supported()?;
        if matches!(selector, RevisionSelector::AssetTag(tag) if &tag.asset != reference) {
            return Err(AppError::new(
                AppErrorCode::InvalidRevision,
                "git.snapshot",
                selector.to_string(),
                "the asset tag belongs to a different asset",
                "Use a tag whose kind and id match the requested asset",
            ));
        }
        let resolved = self.resolve(selector)?;
        match selector {
            RevisionSelector::WorkingTree => self.working_snapshot(reference, resolved.commit),
            _ => self.historical_snapshot(
                reference,
                selector.clone(),
                resolved
                    .commit
                    .expect("committed revisions always resolve to a commit"),
            ),
        }
    }

    pub fn log(
        &self,
        reference: &AssetRef,
        limit: Option<usize>,
    ) -> Result<Vec<GitLogEntry>, AppError> {
        reference.ensure_supported()?;
        if let Err(error) = self.commit_at(&RevisionSelector::Head) {
            if error.code == AppErrorCode::UnknownRevision {
                return Ok(Vec::new());
            }
            return Err(error);
        }
        let limit = limit.map(|value| format!("--max-count={value}"));
        let path = git_path(&reference.root_path());
        let mut args = vec![
            OsStr::new("log"),
            OsStr::new("--format=%H%x1f%an%x1f%aI%x1f%B%x1e"),
        ];
        if let Some(limit) = &limit {
            args.push(OsStr::new(limit));
        }
        args.extend([OsStr::new("--"), OsStr::new(&path)]);
        let output = self.git.output(&self.root, &args, "git.log")?;
        let output = utf8(&output, "git.log", reference.to_string())?;
        let mut entries = Vec::new();
        for record in output.split('\u{1e}') {
            let record = record.trim_matches(['\r', '\n']);
            if record.is_empty() {
                continue;
            }
            let mut fields = record.splitn(4, '\u{1f}');
            let (Some(commit), Some(author), Some(committed_at), Some(message)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                return Err(AppError::new(
                    AppErrorCode::GitCommandFailed,
                    "git.log",
                    reference.to_string(),
                    "Git returned an unrecognized log record",
                    "Retry with a supported Git version",
                ));
            };
            entries.push(GitLogEntry {
                commit: commit.to_owned(),
                author: author.to_owned(),
                committed_at: committed_at.to_owned(),
                message: message.trim_end_matches(['\r', '\n']).to_owned(),
            });
        }
        Ok(entries)
    }

    pub fn diff(
        &self,
        reference: &AssetRef,
        from: &RevisionSelector,
        to: &RevisionSelector,
    ) -> Result<AssetDiff, AppError> {
        let from = self.snapshot(reference, from)?;
        let to = self.snapshot(reference, to)?;
        diff_snapshots(&from, &to)
    }

    pub fn create_tag(
        &self,
        tag: &VersionTag,
        target: &RevisionSelector,
        move_tag: bool,
        confirmed: bool,
    ) -> Result<VersionTag, AppError> {
        let tag = VersionTag::new(tag.asset.clone(), tag.name.clone())?;
        if matches!(target, RevisionSelector::WorkingTree) {
            return Err(AppError::new(
                AppErrorCode::InvalidRevision,
                "git.tag.create",
                tag.to_string(),
                "a tag must point to a committed revision",
                "Use HEAD, a full commit SHA, or another asset tag",
            ));
        }
        let commit = self.commit_at(target)?;
        let exists = self.tag_commit(&tag)?.is_some();
        if exists && tag.is_immutable_version() {
            return Err(AppError::new(
                AppErrorCode::TagAlreadyExists,
                "git.tag.create",
                tag.to_string(),
                "immutable v* tags cannot be overwritten",
                "Choose a new version tag name",
            ));
        }
        if tag.is_release_channel() && (!move_tag || !confirmed) {
            return Err(confirmation_required(&tag, "creating or moving prod"));
        }
        if exists && !move_tag {
            return Err(AppError::new(
                AppErrorCode::TagAlreadyExists,
                "git.tag.create",
                tag.to_string(),
                "the tag already exists",
                "Choose a new name or explicitly request a movable tag",
            ));
        }
        if exists && !confirmed {
            return Err(confirmation_required(&tag, "moving an existing tag"));
        }

        let name = tag.to_string();
        let mut args = vec![OsStr::new("tag")];
        if exists {
            args.push(OsStr::new("--force"));
        }
        args.extend([OsStr::new(&name), OsStr::new(&commit)]);
        self.git.output(&self.root, &args, "git.tag.create")?;
        Ok(tag)
    }

    pub fn list_tags(&self, reference: Option<&AssetRef>) -> Result<Vec<VersionTag>, AppError> {
        if let Some(reference) = reference {
            reference.ensure_supported()?;
        }
        let output = self.git.output(
            &self.root,
            &[
                OsStr::new("for-each-ref"),
                OsStr::new("--format=%(refname:strip=2)"),
                OsStr::new("refs/tags/adock"),
            ],
            "git.tag.list",
        )?;
        let output = utf8(&output, "git.tag.list", "adock")?;
        Ok(output
            .lines()
            .filter_map(|line| VersionTag::parse(line.trim()).ok())
            .filter(|tag| reference.is_none_or(|reference| &tag.asset == reference))
            .collect())
    }

    pub fn rollback_preview(
        &self,
        reference: &AssetRef,
        target: &RevisionSelector,
    ) -> Result<AssetDiff, AppError> {
        self.prepare_rollback(reference, target)
            .map(|preparation| preparation.diff)
    }

    pub fn rollback(
        &self,
        reference: &AssetRef,
        target: &RevisionSelector,
        confirmed: bool,
    ) -> Result<AssetDiff, AppError> {
        let preparation = self.prepare_rollback(reference, target)?;
        if !confirmed {
            return Err(AppError::new(
                AppErrorCode::ConfirmationRequired,
                "git.rollback",
                reference.to_string(),
                "rollback changes working-tree files and requires confirmation",
                "Review the diff preview and explicitly confirm the rollback",
            ));
        }
        for change in &preparation.diff.files {
            let absolute = self.root.join(&change.path);
            match change.change {
                ChangeKind::Removed => match fs::remove_file(&absolute) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(file_write_error(&absolute, error)),
                },
                ChangeKind::Added | ChangeKind::Modified => {
                    let content = preparation
                        .target
                        .files
                        .iter()
                        .find(|file| file.path == change.path)
                        .map(|file| &file.content)
                        .expect("every restored file belongs to the target snapshot");
                    if let Some(parent) = absolute.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|error| file_write_error(parent, error))?;
                    }
                    fs::write(&absolute, content)
                        .map_err(|error| file_write_error(&absolute, error))?;
                }
            }
        }
        Ok(preparation.diff)
    }

    fn commit_at(&self, selector: &RevisionSelector) -> Result<String, AppError> {
        validate_selector(selector)?;
        if matches!(selector, RevisionSelector::WorkingTree) {
            return Err(AppError::new(
                AppErrorCode::InvalidRevision,
                "git.revision.resolve",
                selector.to_string(),
                "the working tree does not identify one commit",
                "Choose HEAD, a full commit SHA, or an asset tag",
            ));
        }
        let revision = match selector {
            RevisionSelector::Head => "HEAD".to_owned(),
            RevisionSelector::Commit(sha) => sha.clone(),
            RevisionSelector::AssetTag(tag) => format!("refs/tags/{tag}"),
            RevisionSelector::WorkingTree => unreachable!(),
        };
        let revision = format!("{revision}^{{commit}}");
        let output = self.git.run(
            &self.root,
            &[
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new("--quiet"),
                OsStr::new(&revision),
            ],
        )?;
        if !output.status.success() {
            return Err(AppError::new(
                AppErrorCode::UnknownRevision,
                "git.revision.resolve",
                selector.to_string(),
                "the revision does not resolve to a commit",
                "Choose an existing commit or asset tag",
            ));
        }
        Ok(utf8(&output.stdout, "git.revision.resolve", revision)?
            .trim()
            .to_owned())
    }

    fn tag_commit(&self, tag: &VersionTag) -> Result<Option<String>, AppError> {
        match self.commit_at(&RevisionSelector::AssetTag(tag.clone())) {
            Ok(commit) => Ok(Some(commit)),
            Err(error) if error.code == AppErrorCode::UnknownRevision => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn working_snapshot(
        &self,
        reference: &AssetRef,
        commit: Option<String>,
    ) -> Result<AssetSnapshot, AppError> {
        let asset = AssetService::new(&self.root).show(reference)?;
        let mut files = Vec::new();
        for path in &asset.source_files {
            let content = fs::read(self.root.join(path)).map_err(|error| {
                AppError::new(
                    AppErrorCode::WorkspaceIo,
                    "git.snapshot.working_tree",
                    path.display().to_string(),
                    error.to_string(),
                    "Check the asset file and retry",
                )
            })?;
            files.push(SnapshotFile {
                path: path.clone(),
                content,
            });
        }
        Ok(AssetSnapshot {
            asset,
            revision: RevisionSelector::WorkingTree,
            commit,
            files,
        })
    }

    fn historical_snapshot(
        &self,
        reference: &AssetRef,
        selector: RevisionSelector,
        commit: String,
    ) -> Result<AssetSnapshot, AppError> {
        let root = git_path(&reference.root_path());
        let output = self.git.output(
            &self.root,
            &[
                OsStr::new("ls-tree"),
                OsStr::new("-r"),
                OsStr::new("-z"),
                OsStr::new("--name-only"),
                OsStr::new(&commit),
                OsStr::new("--"),
                OsStr::new(&root),
            ],
            "git.snapshot.history",
        )?;
        let mut paths = output
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| {
                utf8(path, "git.snapshot.history", reference.to_string())
                    .map(|path| PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        paths.sort();
        let Some(entry_index) = paths
            .iter()
            .position(|path| *path == reference.entry_path())
        else {
            return Err(asset_not_found(reference, &selector));
        };
        let entry = paths.remove(entry_index);
        paths.insert(0, entry);

        let mut files = Vec::new();
        for path in &paths {
            let object = format!("{}:{}", commit, git_path(path));
            let content = self.git.output(
                &self.root,
                &[OsStr::new("show"), OsStr::new(&object)],
                "git.snapshot.history",
            )?;
            files.push(SnapshotFile {
                path: path.clone(),
                content,
            });
        }
        let entry_path = reference.entry_path();
        let input = std::str::from_utf8(&files[0].content).map_err(|error| {
            AppError::new(
                AppErrorCode::InvalidAsset,
                "git.snapshot.history",
                entry_path.display().to_string(),
                format!("asset entry is not UTF-8: {error}"),
                "Store Prompt and Skill entry files as UTF-8 Markdown",
            )
        })?;
        let asset = parse_asset_with_sources(&entry_path, input, paths)?;
        if asset.reference()? != *reference {
            return Err(AppError::new(
                AppErrorCode::InvalidAsset,
                "git.snapshot.history",
                reference.to_string(),
                "the historical asset metadata does not match its path",
                "Choose the AssetRef declared by the historical entry file",
            ));
        }
        Ok(AssetSnapshot {
            asset,
            revision: selector,
            commit: Some(commit),
            files,
        })
    }

    fn prepare_rollback(
        &self,
        reference: &AssetRef,
        target: &RevisionSelector,
    ) -> Result<RollbackPreparation, AppError> {
        reference.ensure_supported()?;
        if matches!(target, RevisionSelector::WorkingTree) {
            return Err(AppError::new(
                AppErrorCode::InvalidRevision,
                "git.rollback",
                reference.to_string(),
                "working-tree cannot be a rollback target",
                "Choose HEAD, a full commit SHA, or an asset tag",
            ));
        }
        let target_snapshot = self.snapshot(reference, target)?;
        self.ensure_clean(reference)?;
        let current = match self.snapshot(reference, &RevisionSelector::WorkingTree) {
            Ok(snapshot) => snapshot,
            Err(error) if error.code == AppErrorCode::AssetNotFound => AssetSnapshot {
                asset: target_snapshot.asset.clone(),
                revision: RevisionSelector::WorkingTree,
                commit: self.resolve(&RevisionSelector::WorkingTree)?.commit,
                files: Vec::new(),
            },
            Err(error) => return Err(error),
        };
        Ok(RollbackPreparation {
            diff: diff_snapshots(&current, &target_snapshot)?,
            target: target_snapshot,
        })
    }

    fn ensure_clean(&self, reference: &AssetRef) -> Result<(), AppError> {
        let path = git_path(&reference.root_path());
        let output = self.git.output(
            &self.root,
            &[
                OsStr::new("status"),
                OsStr::new("--porcelain=v1"),
                OsStr::new("-z"),
                OsStr::new("--untracked-files=all"),
                OsStr::new("--"),
                OsStr::new(&path),
            ],
            "git.rollback.status",
        )?;
        if output.is_empty() {
            Ok(())
        } else {
            Err(AppError::new(
                AppErrorCode::DirtyAsset,
                "git.rollback",
                reference.to_string(),
                "the target asset has uncommitted changes or untracked files",
                "Commit, stash, or discard only the target asset changes before rollback",
            ))
        }
    }
}

struct RollbackPreparation {
    target: AssetSnapshot,
    diff: AssetDiff,
}

fn validate_selector(selector: &RevisionSelector) -> Result<(), AppError> {
    match selector {
        RevisionSelector::WorkingTree | RevisionSelector::Head => Ok(()),
        RevisionSelector::Commit(sha) => RevisionSelector::parse(sha).map(|_| ()),
        RevisionSelector::AssetTag(tag) => {
            VersionTag::new(tag.asset.clone(), tag.name.clone()).map(|_| ())
        }
    }
}

fn confirmation_required(tag: &VersionTag, action: &str) -> AppError {
    AppError::new(
        AppErrorCode::ConfirmationRequired,
        "git.tag.create",
        tag.to_string(),
        format!("{action} requires both explicit move and confirmation"),
        "Pass the move option and confirm the operation; no tag is pushed automatically",
    )
}

fn asset_not_found(reference: &AssetRef, selector: &RevisionSelector) -> AppError {
    AppError::new(
        AppErrorCode::AssetNotFound,
        "git.snapshot.history",
        reference.to_string(),
        format!("asset does not exist at {selector}"),
        "Choose a revision where the asset entry exists",
    )
}

fn file_write_error(path: &Path, error: std::io::Error) -> AppError {
    AppError::new(
        AppErrorCode::FileWrite,
        "git.rollback.write",
        path.display().to_string(),
        error.to_string(),
        "Check file permissions and retry; no commit or push is performed",
    )
}

fn command_failed(operation: &str, root: &Path, output: &Output) -> AppError {
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let reason = if detail.is_empty() {
        format!("Git exited with status {}", output.status)
    } else {
        detail
    };
    AppError::new(
        AppErrorCode::GitCommandFailed,
        operation,
        root.display().to_string(),
        reason,
        "Check the repository state and retry the Git operation",
    )
}

fn utf8<'a>(
    bytes: &'a [u8],
    operation: &str,
    subject: impl Into<String>,
) -> Result<&'a str, AppError> {
    std::str::from_utf8(bytes).map_err(|error| {
        AppError::new(
            AppErrorCode::GitCommandFailed,
            operation,
            subject,
            format!("Git returned non-UTF-8 data: {error}"),
            "Use UTF-8 asset paths and Git metadata",
        )
    })
}

fn git_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use adock_core::AssetKind;
    use tempfile::TempDir;

    use super::*;

    struct Fixture {
        _temp: TempDir,
        root: PathBuf,
        repository: GitRepository,
        first: String,
        second: String,
        prompt: AssetRef,
        skill: AssetRef,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let root = temp.path().to_path_buf();
            git(&root, &["init", "-q"]);
            git(&root, &["config", "user.name", "Agent Dock Tests"]);
            git(&root, &["config", "user.email", "tests@example.invalid"]);
            fs::create_dir_all(root.join("prompts")).unwrap();
            fs::create_dir_all(root.join("skills/onboard/scripts")).unwrap();
            fs::write(root.join("prompts/review.md"), prompt("Review first\n")).unwrap();
            fs::write(root.join("skills/onboard/SKILL.md"), skill("Read first\n")).unwrap();
            fs::write(root.join("skills/onboard/scripts/run.sh"), "echo first\n").unwrap();
            fs::write(root.join("skills/onboard/obsolete.txt"), "remove me\n").unwrap();
            git(&root, &["add", "."]);
            git(&root, &["commit", "-q", "-m", "first assets"]);
            let first = head(&root);

            fs::write(root.join("prompts/review.md"), prompt("Review second\n")).unwrap();
            fs::write(root.join("skills/onboard/SKILL.md"), skill("Read second\n")).unwrap();
            fs::write(root.join("skills/onboard/scripts/run.sh"), "echo second\n").unwrap();
            fs::write(root.join("skills/onboard/blob.bin"), [0, 159, 146, 150]).unwrap();
            fs::remove_file(root.join("skills/onboard/obsolete.txt")).unwrap();
            git(&root, &["add", "-A"]);
            git(&root, &["commit", "-q", "-m", "second assets"]);
            let second = head(&root);
            let repository = GitRepository::open(&root).unwrap();
            Self {
                _temp: temp,
                root,
                repository,
                first,
                second,
                prompt: AssetRef::parse("prompt:review").unwrap(),
                skill: AssetRef::parse("skill:onboard").unwrap(),
            }
        }
    }

    fn prompt(body: &str) -> String {
        format!(
            "---\napiVersion: adock/v1alpha1\nkind: prompt\nid: review\ntitle: Review\ndescription: Review code\nowners: [alice]\nspec:\n  variables: []\n---\n{body}"
        )
    }

    fn skill(body: &str) -> String {
        format!(
            "---\napiVersion: adock/v1alpha1\nkind: skill\nid: onboard\ntitle: Onboard\ndescription: Onboard a repository\nowners: [bob]\nspec:\n  triggers: []\n  dependencies: []\n---\n{body}"
        )
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

    fn head(root: &Path) -> String {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn reads_historical_snapshots_logs_and_multifile_diffs_without_checkout() {
        let fixture = Fixture::new();
        let first = RevisionSelector::Commit(fixture.first.clone());
        let historical = fixture.repository.snapshot(&fixture.skill, &first).unwrap();
        assert_eq!(historical.commit.as_deref(), Some(fixture.first.as_str()));
        assert_eq!(historical.asset.body, "Read first\n");
        assert_eq!(historical.files.len(), 3);

        let diff = fixture
            .repository
            .diff(&fixture.skill, &first, &RevisionSelector::Head)
            .unwrap();
        assert!(diff.files.iter().any(|change| {
            change.path.ends_with("scripts/run.sh") && change.change == ChangeKind::Modified
        }));
        assert!(diff.files.iter().any(|change| {
            change.path.ends_with("obsolete.txt") && change.change == ChangeKind::Removed
        }));
        assert!(diff.files.iter().any(|change| {
            change.path.ends_with("blob.bin") && change.change == ChangeKind::Added && change.binary
        }));
        assert_eq!(head(&fixture.root), fixture.second);

        let working = fixture
            .repository
            .snapshot(&fixture.prompt, &RevisionSelector::WorkingTree)
            .unwrap();
        assert_eq!(working.asset.body, "Review second\n");
        let log = fixture.repository.log(&fixture.prompt, None).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].commit, fixture.second);
        assert_eq!(log[0].author, "Agent Dock Tests");
        assert_eq!(log[0].message, "second assets");
        assert!(!log[0].committed_at.is_empty());
        assert_eq!(
            fixture
                .repository
                .log(&fixture.prompt, Some(1))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn enforces_tag_immutability_and_explicit_prod_moves() {
        let fixture = Fixture::new();
        let v1 = VersionTag::new(fixture.prompt.clone(), "v1.0.0").unwrap();
        fixture
            .repository
            .create_tag(&v1, &RevisionSelector::Head, false, false)
            .unwrap();
        let error = fixture
            .repository
            .create_tag(
                &v1,
                &RevisionSelector::Commit(fixture.first.clone()),
                true,
                true,
            )
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::TagAlreadyExists);

        let prod = VersionTag::new(fixture.prompt.clone(), "prod").unwrap();
        for (move_tag, confirmed) in [(false, false), (true, false), (false, true)] {
            let error = fixture
                .repository
                .create_tag(&prod, &RevisionSelector::Head, move_tag, confirmed)
                .unwrap_err();
            assert_eq!(error.code, AppErrorCode::ConfirmationRequired);
        }
        fixture
            .repository
            .create_tag(&prod, &RevisionSelector::Head, true, true)
            .unwrap();
        fixture
            .repository
            .create_tag(
                &prod,
                &RevisionSelector::Commit(fixture.first.clone()),
                true,
                true,
            )
            .unwrap();
        assert_eq!(
            fixture
                .repository
                .resolve(&RevisionSelector::AssetTag(prod.clone()))
                .unwrap()
                .commit
                .as_deref(),
            Some(fixture.first.as_str())
        );
        assert_eq!(
            fixture
                .repository
                .list_tags(Some(&fixture.prompt))
                .unwrap()
                .len(),
            2
        );
        assert!(
            fixture
                .repository
                .list_tags(Some(&fixture.skill))
                .unwrap()
                .is_empty()
        );
        assert_eq!(head(&fixture.root), fixture.second);
    }

    #[test]
    fn rollback_restores_only_the_target_asset_and_blocks_dirty_target_paths() {
        let fixture = Fixture::new();
        fs::write(
            fixture.root.join("prompts/review.md"),
            prompt("Local prompt\n"),
        )
        .unwrap();
        let target = RevisionSelector::Commit(fixture.first.clone());
        let preview = fixture
            .repository
            .rollback_preview(&fixture.skill, &target)
            .unwrap();
        assert!(!preview.is_empty());
        let error = fixture
            .repository
            .rollback(&fixture.skill, &target, false)
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::ConfirmationRequired);

        fixture
            .repository
            .rollback(&fixture.skill, &target, true)
            .unwrap();
        assert_eq!(
            fs::read_to_string(fixture.root.join("skills/onboard/SKILL.md")).unwrap(),
            skill("Read first\n")
        );
        assert!(!fixture.root.join("skills/onboard/blob.bin").exists());
        assert!(fixture.root.join("skills/onboard/obsolete.txt").exists());
        assert_eq!(
            fs::read_to_string(fixture.root.join("prompts/review.md")).unwrap(),
            prompt("Local prompt\n")
        );
        assert_eq!(head(&fixture.root), fixture.second);

        let error = fixture
            .repository
            .rollback_preview(&fixture.skill, &RevisionSelector::Head)
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::DirtyAsset);
    }

    #[test]
    fn reports_repository_revision_and_git_availability_errors() {
        let temp = TempDir::new().unwrap();
        let error = GitRepository::open(temp.path()).unwrap_err();
        assert_eq!(error.code, AppErrorCode::NotGitRepository);
        let error = GitRepository::open_with_git(
            temp.path(),
            GitCli::with_executable("adock-test-git-that-does-not-exist"),
        )
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::GitUnavailable);

        let fixture = Fixture::new();
        let error = fixture
            .repository
            .snapshot(
                &fixture.prompt,
                &RevisionSelector::Commit("0123456789abcdef0123456789abcdef01234567".to_owned()),
            )
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::UnknownRevision);
        let missing = AssetRef::parse("prompt:missing").unwrap();
        let error = fixture
            .repository
            .snapshot(&missing, &RevisionSelector::Head)
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::AssetNotFound);
        let tag = VersionTag::new(fixture.skill.clone(), "v9.0.0").unwrap();
        let error = fixture
            .repository
            .snapshot(&fixture.prompt, &RevisionSelector::AssetTag(tag))
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidRevision);
        assert_eq!(fixture.prompt.kind, AssetKind::Prompt);
    }
}
