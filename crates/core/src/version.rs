use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{AppError, AppErrorCode, AssetRef, AssetSnapshot, RevisionSelector, SnapshotFile};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: PathBuf,
    pub change: ChangeKind,
    pub binary: bool,
    pub text_diff: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetDiff {
    pub asset: AssetRef,
    pub from: RevisionSelector,
    pub to: RevisionSelector,
    pub from_commit: Option<String>,
    pub to_commit: Option<String>,
    pub files: Vec<FileChange>,
}

impl AssetDiff {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

pub fn diff_snapshots(from: &AssetSnapshot, to: &AssetSnapshot) -> Result<AssetDiff, AppError> {
    let from_ref = from.asset.reference()?;
    let to_ref = to.asset.reference()?;
    if from_ref != to_ref {
        return Err(AppError::new(
            AppErrorCode::InvalidInput,
            "asset.diff",
            format!("{from_ref} vs {to_ref}"),
            "snapshots belong to different assets",
            "Compare two snapshots of the same AssetRef",
        ));
    }
    let from_files = from.files.iter().map(file_entry).collect();
    let to_files = to.files.iter().map(file_entry).collect();
    Ok(AssetDiff {
        asset: from_ref,
        from: from.revision.clone(),
        to: to.revision.clone(),
        from_commit: from.commit.clone(),
        to_commit: to.commit.clone(),
        files: build_changes(&from_files, &to_files),
    })
}

fn file_entry(file: &SnapshotFile) -> (PathBuf, &[u8]) {
    (file.path.clone(), &file.content)
}

fn build_changes(
    from: &BTreeMap<PathBuf, &[u8]>,
    to: &BTreeMap<PathBuf, &[u8]>,
) -> Vec<FileChange> {
    let paths: std::collections::BTreeSet<_> = from.keys().chain(to.keys()).collect();
    paths
        .into_iter()
        .filter_map(|path| {
            let old = from.get(path).copied();
            let new = to.get(path).copied();
            if old == new {
                return None;
            }
            let change = match (old, new) {
                (None, Some(_)) => ChangeKind::Added,
                (Some(_), None) => ChangeKind::Removed,
                _ => ChangeKind::Modified,
            };
            let binary = old.is_some_and(is_binary) || new.is_some_and(is_binary);
            let text_diff = if binary {
                None
            } else {
                Some(unified_text_diff(
                    path,
                    old.map(|bytes| std::str::from_utf8(bytes).unwrap_or_default()),
                    new.map(|bytes| std::str::from_utf8(bytes).unwrap_or_default()),
                ))
            };
            Some(FileChange {
                path: path.clone(),
                change,
                binary,
                text_diff,
            })
        })
        .collect()
}

fn is_binary(content: &[u8]) -> bool {
    content.contains(&0) || std::str::from_utf8(content).is_err()
}

fn unified_text_diff(path: &Path, old: Option<&str>, new: Option<&str>) -> String {
    let old_lines = old.unwrap_or_default().lines().collect::<Vec<_>>();
    let new_lines = new.unwrap_or_default().lines().collect::<Vec<_>>();
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_lines.len().saturating_sub(prefix)
        && suffix < new_lines.len().saturating_sub(prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mut output = format!(
        "--- {}\n+++ {}\n@@ -{},{} +{},{} @@\n",
        if old.is_some() {
            format!("a/{}", path.display())
        } else {
            "/dev/null".to_owned()
        },
        if new.is_some() {
            format!("b/{}", path.display())
        } else {
            "/dev/null".to_owned()
        },
        prefix + 1,
        old_lines.len() - prefix - suffix,
        prefix + 1,
        new_lines.len() - prefix - suffix,
    );
    for line in &old_lines[..prefix] {
        output.push_str(&format!(" {line}\n"));
    }
    for line in &old_lines[prefix..old_lines.len() - suffix] {
        output.push_str(&format!("-{line}\n"));
    }
    for line in &new_lines[prefix..new_lines.len() - suffix] {
        output.push_str(&format!("+{line}\n"));
    }
    for line in &old_lines[old_lines.len() - suffix..] {
        output.push_str(&format!(" {line}\n"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_text_and_binary_resource_changes() {
        let text = PathBuf::from("skills/demo/reference.md");
        let binary = PathBuf::from("skills/demo/blob.bin");
        let added = PathBuf::from("skills/demo/new.txt");
        let from = BTreeMap::from([
            (text.clone(), "old\nsame\n".as_bytes()),
            (binary.clone(), &[0, 1][..]),
        ]);
        let to = BTreeMap::from([
            (text.clone(), "new\nsame\n".as_bytes()),
            (binary.clone(), &[0, 2][..]),
            (added.clone(), "added\n".as_bytes()),
        ]);
        let changes = build_changes(&from, &to);
        assert_eq!(changes.len(), 3);
        assert!(changes.iter().any(|change| change.binary));
        assert_eq!(
            changes
                .iter()
                .find(|change| change.path == added)
                .unwrap()
                .change,
            ChangeKind::Added
        );
        let text_change = changes.iter().find(|change| change.path == text).unwrap();
        assert!(text_change.text_diff.as_ref().unwrap().contains("-old"));
        assert!(text_change.text_diff.as_ref().unwrap().contains("+new"));
    }
}
