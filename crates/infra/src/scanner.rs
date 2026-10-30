use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use adock_core::{AppError, AppErrorCode, Asset, AssetKind, AssetRef, parse_asset_with_sources};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanIssue {
    pub path: PathBuf,
    pub asset: Option<AssetRef>,
    pub error: AppError,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanResult {
    pub assets: Vec<Asset>,
    pub issues: Vec<ScanIssue>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceScanner {
    root: PathBuf,
}

impl WorkspaceScanner {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn scan(&self) -> Result<ScanResult, AppError> {
        let root = self.root.canonicalize().map_err(|error| {
            AppError::new(
                AppErrorCode::WorkspaceIo,
                "assets.scan",
                self.root.display().to_string(),
                error.to_string(),
                "Choose an existing workspace directory",
            )
        })?;
        let mut result = ScanResult::default();
        self.scan_prompts(&root, &mut result);
        self.scan_skills(&root, &mut result);

        let mut grouped: BTreeMap<AssetRef, Vec<Asset>> = BTreeMap::new();
        for asset in result.assets {
            match asset.reference() {
                Ok(reference) => grouped.entry(reference).or_default().push(asset),
                Err(error) => result.issues.push(ScanIssue {
                    path: asset.source_files.first().cloned().unwrap_or_default(),
                    asset: None,
                    error,
                }),
            }
        }
        let mut assets = Vec::new();
        for (reference, group) in grouped {
            if group.len() == 1 {
                assets.push(group.into_iter().next().expect("singleton group"));
                continue;
            }
            let locations = group
                .iter()
                .filter_map(|asset| asset.source_files.first())
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            for asset in group {
                let path = asset.source_files.first().cloned().unwrap_or_default();
                result.issues.push(ScanIssue {
                    path,
                    asset: Some(reference.clone()),
                    error: AppError::new(
                        AppErrorCode::DuplicateAsset,
                        "assets.scan",
                        reference.to_string(),
                        format!("duplicate AssetRef found at {locations}"),
                        "Keep exactly one asset with this kind and id",
                    ),
                });
            }
        }
        result.assets = assets;
        result.assets.sort_by_key(|asset| asset.reference().ok());
        result
            .issues
            .sort_by(|left, right| left.path.cmp(&right.path));
        Ok(result)
    }

    fn scan_prompts(&self, root: &Path, result: &mut ScanResult) {
        let directory = root.join("prompts");
        let entries = match read_directory(&directory, result) {
            Some(entries) => entries,
            None => return,
        };
        for path in entries {
            if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
                continue;
            }
            let relative = relative_path(root, &path);
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    push_io_issue(result, relative, "symbolic-link assets are not supported");
                    continue;
                }
                Ok(metadata) if metadata.is_file() => {}
                Ok(_) => continue,
                Err(error) => {
                    push_io_issue(result, relative, error.to_string());
                    continue;
                }
            }
            match fs::read_to_string(&path) {
                Ok(input) => {
                    match parse_asset_with_sources(&relative, &input, vec![relative.clone()]) {
                        Ok(asset) if asset.metadata.kind == AssetKind::Prompt => {
                            result.assets.push(asset)
                        }
                        Ok(asset) => result.issues.push(ScanIssue {
                            path: relative.clone(),
                            asset: asset.reference().ok(),
                            error: AppError::new(
                                AppErrorCode::InvalidAsset,
                                "assets.scan",
                                relative.display().to_string(),
                                "a file under prompts must have kind: prompt",
                                "Move the asset to the correct directory or change its kind",
                            ),
                        }),
                        Err(error) => result.issues.push(ScanIssue {
                            path: relative,
                            asset: None,
                            error,
                        }),
                    }
                }
                Err(error) => push_io_issue(result, relative, error.to_string()),
            }
        }
    }

    fn scan_skills(&self, root: &Path, result: &mut ScanResult) {
        let directory = root.join("skills");
        let entries = match read_directory(&directory, result) {
            Some(entries) => entries,
            None => return,
        };
        for skill_root in entries {
            let Ok(metadata) = fs::symlink_metadata(&skill_root) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                push_io_issue(
                    result,
                    relative_path(root, &skill_root),
                    "symbolic-link Skill directories are not supported",
                );
                continue;
            }
            if !metadata.is_dir() {
                continue;
            }
            let entry = skill_root.join("SKILL.md");
            let relative_entry = relative_path(root, &entry);
            if !entry.is_file() {
                result.issues.push(ScanIssue {
                    path: relative_entry.clone(),
                    asset: None,
                    error: AppError::new(
                        AppErrorCode::AssetNotFound,
                        "assets.scan",
                        relative_entry.display().to_string(),
                        "Skill directory is missing SKILL.md",
                        "Add a valid SKILL.md entry file",
                    ),
                });
                continue;
            }
            let (mut source_files, mut issues, valid) = collect_source_files(root, &skill_root);
            result.issues.append(&mut issues);
            if !valid {
                continue;
            }
            source_files.sort();
            if let Some(index) = source_files.iter().position(|path| *path == relative_entry) {
                let entry_path = source_files.remove(index);
                source_files.insert(0, entry_path);
            }
            match fs::read_to_string(&entry) {
                Ok(input) => {
                    match parse_asset_with_sources(&relative_entry, &input, source_files) {
                        Ok(asset) if asset.metadata.kind == AssetKind::Skill => {
                            result.assets.push(asset)
                        }
                        Ok(asset) => result.issues.push(ScanIssue {
                            path: relative_entry.clone(),
                            asset: asset.reference().ok(),
                            error: AppError::new(
                                AppErrorCode::InvalidAsset,
                                "assets.scan",
                                relative_entry.display().to_string(),
                                "SKILL.md must have kind: skill",
                                "Change the asset kind or move it to prompts",
                            ),
                        }),
                        Err(error) => result.issues.push(ScanIssue {
                            path: relative_entry,
                            asset: None,
                            error,
                        }),
                    }
                }
                Err(error) => push_io_issue(result, relative_entry, error.to_string()),
            }
        }
    }
}

fn read_directory(path: &Path, result: &mut ScanResult) -> Option<Vec<PathBuf>> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            push_io_issue(result, path.to_path_buf(), error.to_string());
            return None;
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(error) => push_io_issue(result, path.to_path_buf(), error.to_string()),
        }
    }
    paths.sort();
    Some(paths)
}

fn collect_source_files(root: &Path, directory: &Path) -> (Vec<PathBuf>, Vec<ScanIssue>, bool) {
    let mut files = Vec::new();
    let mut issues = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    let mut valid = true;
    while let Some(current) = pending.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(error) => {
                valid = false;
                issues.push(issue(relative_path(root, &current), error.to_string()));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    valid = false;
                    issues.push(issue(relative_path(root, &current), error.to_string()));
                    continue;
                }
            };
            let path = entry.path();
            match entry.file_type() {
                Ok(file_type) if file_type.is_symlink() => {
                    valid = false;
                    issues.push(issue(
                        relative_path(root, &path),
                        "symbolic links are not supported inside Skill assets",
                    ));
                }
                Ok(file_type) if file_type.is_dir() => pending.push(path),
                Ok(file_type) if file_type.is_file() => files.push(relative_path(root, &path)),
                Ok(_) => {}
                Err(error) => {
                    valid = false;
                    issues.push(issue(relative_path(root, &path), error.to_string()));
                }
            }
        }
    }
    (files, issues, valid)
}

fn issue(path: PathBuf, reason: impl Into<String>) -> ScanIssue {
    ScanIssue {
        path: path.clone(),
        asset: None,
        error: AppError::new(
            AppErrorCode::InvalidAsset,
            "assets.scan",
            path.display().to_string(),
            reason,
            "Correct the asset source files and retry",
        ),
    }
}

fn push_io_issue(result: &mut ScanResult, path: PathBuf, reason: impl Into<String>) {
    result.issues.push(issue(path, reason));
}

fn relative_path(root: &Path, path: &Path) -> PathBuf {
    let Ok(relative) = path.strip_prefix(root) else {
        return path.to_path_buf();
    };
    let portable = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    PathBuf::from(portable)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use adock_core::AssetSpec;
    use tempfile::TempDir;

    use super::*;

    fn prompt(id: &str) -> String {
        format!(
            "---\napiVersion: adock/v1alpha1\nkind: prompt\nid: {id}\ntitle: Review\ndescription: Review code\nowners: [alice]\nspec:\n  variables: []\n---\nReview code\n"
        )
    }

    fn skill(id: &str) -> String {
        format!(
            "---\napiVersion: adock/v1alpha1\nkind: skill\nid: {id}\ntitle: Onboarding\ndescription: Onboard a repository\nowners: [bob]\nspec:\n  triggers: [new repository]\n  dependencies: [prompt:review]\n  entrypoint: scripts/run.sh\n---\nRead the references.\n"
        )
    }

    #[test]
    fn scans_prompt_and_multifile_skill() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("prompts")).unwrap();
        fs::create_dir_all(temp.path().join("skills/onboard/scripts")).unwrap();
        fs::create_dir_all(temp.path().join("skills/onboard/references")).unwrap();
        fs::write(temp.path().join("prompts/review.md"), prompt("review")).unwrap();
        fs::write(
            temp.path().join("skills/onboard/SKILL.md"),
            skill("onboard"),
        )
        .unwrap();
        fs::write(
            temp.path().join("skills/onboard/scripts/run.sh"),
            "exit 0\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("skills/onboard/references/repo.md"),
            "Repository\n",
        )
        .unwrap();

        let result = WorkspaceScanner::new(temp.path()).scan().unwrap();
        assert_eq!(result.assets.len(), 2);
        assert!(result.issues.is_empty());
        let skill = result
            .assets
            .iter()
            .find(|asset| asset.metadata.kind == AssetKind::Skill)
            .unwrap();
        assert_eq!(skill.source_files.len(), 3);
        assert!(
            skill
                .source_files
                .iter()
                .all(|path| !path.to_string_lossy().contains('\\'))
        );
        assert!(matches!(&skill.spec, AssetSpec::Skill(spec) if spec.entrypoint.is_some()));
    }

    #[test]
    fn isolates_broken_duplicate_and_missing_entry_assets() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("prompts")).unwrap();
        fs::create_dir_all(temp.path().join("skills/broken")).unwrap();
        fs::create_dir_all(temp.path().join("skills/missing")).unwrap();
        fs::write(temp.path().join("prompts/good.md"), prompt("good")).unwrap();
        fs::write(temp.path().join("prompts/bad.md"), "not front matter").unwrap();
        fs::write(temp.path().join("skills/broken/SKILL.md"), skill("good")).unwrap();
        fs::write(temp.path().join("prompts/duplicate.md"), prompt("good")).unwrap();

        let result = WorkspaceScanner::new(temp.path()).scan().unwrap();
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].metadata.kind, AssetKind::Skill);
        assert_eq!(
            result
                .issues
                .iter()
                .filter(|issue| issue.error.code == AppErrorCode::DuplicateAsset)
                .count(),
            2
        );
        assert!(
            result
                .issues
                .iter()
                .any(|issue| issue.path.ends_with("skills/missing/SKILL.md"))
        );
        fs::remove_file(temp.path().join("prompts/duplicate.md")).unwrap();
        fs::remove_dir_all(temp.path().join("skills/broken")).unwrap();
        let result = WorkspaceScanner::new(temp.path()).scan().unwrap();
        assert_eq!(result.assets.len(), 1);
    }
}
