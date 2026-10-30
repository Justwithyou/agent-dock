mod config;
mod distribution;
mod init;
mod scanner;
mod service;

use std::{fs, path::Path};

use adock_core::{AppError, AppErrorCode, WorkspaceDiscovery, WorkspaceInfo};

pub use config::{LocalWorkspaceConfig, TargetConfig, WorkspaceConfig, load_workspace_config};
pub use distribution::{DistributionManifest, DistributionManifestEntry, DistributionService};
pub use init::WorkspaceInitializer;
pub use scanner::{ScanIssue, ScanResult, WorkspaceScanner};
pub use service::{AssetFilter, AssetListItem, AssetListResult, AssetService};

pub const WORKSPACE_CONFIG_FILE: &str = "adock.yaml";
pub const LOCAL_WORKSPACE_CONFIG_FILE: &str = "adock.local.yaml";

#[derive(Debug, Clone, Copy, Default)]
pub struct FileSystemWorkspaceDiscovery;

impl WorkspaceDiscovery for FileSystemWorkspaceDiscovery {
    fn discover(&self, start: &Path) -> Result<WorkspaceInfo, AppError> {
        let start = start.canonicalize().map_err(|error| {
            AppError::new(
                AppErrorCode::WorkspaceIo,
                "workspace.discover",
                start.display().to_string(),
                error.to_string(),
                "Choose an existing directory",
            )
        })?;

        if !start.is_dir() {
            return Err(AppError::new(
                AppErrorCode::InvalidInput,
                "workspace.discover",
                start.display().to_string(),
                "the starting path is not a directory",
                "Choose a directory inside the asset workspace",
            ));
        }

        for directory in start.ancestors() {
            let config_path = directory.join(WORKSPACE_CONFIG_FILE);
            match fs::metadata(&config_path) {
                Ok(metadata) if metadata.is_file() => {
                    let git_root = directory
                        .ancestors()
                        .find(|ancestor| ancestor.join(".git").exists())
                        .map(Path::to_path_buf)
                        .ok_or_else(|| {
                            AppError::new(
                                AppErrorCode::NotGitRepository,
                                "workspace.discover",
                                directory.display().to_string(),
                                "workspace configuration was found outside a Git repository",
                                "Initialize or clone a Git repository before opening this workspace",
                            )
                        })?;
                    return Ok(WorkspaceInfo {
                        root: directory.to_path_buf(),
                        config_path,
                        git_root: Some(git_root),
                    });
                }
                Ok(_) => {
                    return Err(AppError::new(
                        AppErrorCode::WorkspaceIo,
                        "workspace.discover",
                        config_path.display().to_string(),
                        "the workspace configuration path is not a file",
                        "Replace it with a valid adock.yaml file",
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(AppError::new(
                        AppErrorCode::WorkspaceIo,
                        "workspace.discover",
                        config_path.display().to_string(),
                        error.to_string(),
                        "Check the configuration path permissions and retry",
                    ));
                }
            }
        }

        Err(AppError::new(
            AppErrorCode::WorkspaceNotFound,
            "workspace.discover",
            start.display().to_string(),
            format!("no {WORKSPACE_CONFIG_FILE} found in this directory or its ancestors"),
            "Choose an asset workspace or create adock.yaml at its root",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "adock-infra-{}-{timestamp}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovers_workspace_configuration_in_ancestors() {
        let temp = TestDirectory::new();
        let root = temp.0.join("assets");
        let nested = root.join("skills").join("repo-onboarding");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(WORKSPACE_CONFIG_FILE), "").unwrap();

        let info = FileSystemWorkspaceDiscovery.discover(&nested).unwrap();

        assert_eq!(info.root, root.canonicalize().unwrap());
        assert_eq!(
            info.config_path,
            root.canonicalize().unwrap().join(WORKSPACE_CONFIG_FILE)
        );
    }

    #[test]
    fn reports_missing_workspace_configuration() {
        let temp = TestDirectory::new();
        let nested = temp.0.join("without-config").join("nested");
        fs::create_dir_all(&nested).unwrap();

        let error = FileSystemWorkspaceDiscovery.discover(&nested).unwrap_err();

        assert_eq!(error.code, AppErrorCode::WorkspaceNotFound);
        assert!(error.reason.contains(WORKSPACE_CONFIG_FILE));
    }
}
