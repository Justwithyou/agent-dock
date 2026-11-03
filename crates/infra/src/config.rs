use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use adock_core::{AppError, AppErrorCode, TargetProfile};
use serde::{Deserialize, Serialize};

use crate::{LOCAL_WORKSPACE_CONFIG_FILE, WORKSPACE_CONFIG_FILE};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    #[serde(default = "default_channel")]
    pub default_channel: String,
    #[serde(default)]
    pub targets: BTreeMap<String, TargetConfig>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            default_channel: default_channel(),
            targets: BTreeMap::new(),
        }
    }
}

impl WorkspaceConfig {
    pub fn target_profile(
        &self,
        workspace_root: &Path,
        target: &str,
    ) -> Result<TargetProfile, AppError> {
        let configured = self.targets.get(target).ok_or_else(|| {
            AppError::new(
                AppErrorCode::InvalidConfig,
                "config.target.resolve",
                target,
                "target profile is not configured",
                "Add the target to adock.yaml or adock.local.yaml",
            )
        })?;
        let root = if configured.root.is_absolute() {
            configured.root.clone()
        } else {
            workspace_root.join(&configured.root)
        };
        TargetProfile::new(target, root)
    }
}

fn default_channel() -> String {
    "prod".to_owned()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalWorkspaceConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_channel: Option<String>,
    #[serde(default)]
    pub targets: BTreeMap<String, TargetConfig>,
}

pub fn load_workspace_config(root: &Path) -> Result<WorkspaceConfig, AppError> {
    let shared_path = root.join(WORKSPACE_CONFIG_FILE);
    let shared: WorkspaceConfig = read_yaml(&shared_path)?;
    let local_path = root.join(LOCAL_WORKSPACE_CONFIG_FILE);
    let local = read_local_config(&local_path)?;
    merge_configs(shared, local, home_dir().as_deref())
}

pub fn save_local_target(
    workspace_root: &Path,
    target: &str,
    root: PathBuf,
) -> Result<(), AppError> {
    let shared: WorkspaceConfig = read_yaml(&workspace_root.join(WORKSPACE_CONFIG_FILE))?;
    let local_path = workspace_root.join(LOCAL_WORKSPACE_CONFIG_FILE);
    let mut local = read_local_config(&local_path)?;
    local
        .targets
        .insert(target.to_owned(), TargetConfig { root });
    merge_configs(shared, local.clone(), home_dir().as_deref())?;
    let yaml = serde_yaml::to_string(&local)
        .map_err(|error| config_error(&local_path, error.to_string()))?;
    fs::write(&local_path, yaml).map_err(|error| config_error(&local_path, error.to_string()))?;
    load_workspace_config(workspace_root)?;
    Ok(())
}

fn read_local_config(path: &Path) -> Result<LocalWorkspaceConfig, AppError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => read_yaml(path),
        Ok(_) => Err(config_error(path, "local configuration path is not a file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(LocalWorkspaceConfig::default())
        }
        Err(error) => Err(config_error(path, error.to_string())),
    }
}

fn merge_configs(
    shared: WorkspaceConfig,
    local: LocalWorkspaceConfig,
    home: Option<&Path>,
) -> Result<WorkspaceConfig, AppError> {
    let mut targets = shared.targets;
    for (name, target) in local.targets {
        targets.insert(name, target);
    }
    let default_channel = local.default_channel.unwrap_or(shared.default_channel);
    if default_channel.trim().is_empty()
        || default_channel.trim() != default_channel
        || default_channel.contains('/')
    {
        return Err(AppError::new(
            AppErrorCode::InvalidConfig,
            "config.merge",
            "default_channel",
            "default channel must be a non-empty tag name without slashes",
            "Use prod or another valid channel name",
        ));
    }
    let mut expanded = BTreeMap::new();
    for (name, target) in targets {
        if name.trim().is_empty() || name.trim() != name {
            return Err(AppError::new(
                AppErrorCode::InvalidConfig,
                "config.merge",
                name,
                "target names must be non-empty and have no surrounding whitespace",
                "Correct the target name in configuration",
            ));
        }
        if target.root.as_os_str().is_empty() {
            return Err(AppError::new(
                AppErrorCode::InvalidConfig,
                "config.merge",
                format!("targets.{name}.root"),
                "target root cannot be empty",
                "Set a target root path",
            ));
        }
        expanded.insert(
            name,
            TargetConfig {
                root: expand_home(&target.root, home)?,
            },
        );
    }
    Ok(WorkspaceConfig {
        default_channel,
        targets: expanded,
    })
}

fn expand_home(path: &Path, home: Option<&Path>) -> Result<PathBuf, AppError> {
    let value = path.to_string_lossy();
    if value == "~" {
        return home.map(Path::to_path_buf).ok_or_else(home_error);
    }
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        return home.map(|home| home.join(rest)).ok_or_else(home_error);
    }
    Ok(path.to_path_buf())
}

fn home_error() -> AppError {
    AppError::new(
        AppErrorCode::InvalidConfig,
        "config.expand_home",
        "~",
        "cannot expand ~ because the home directory is unavailable",
        "Set the home directory environment variable or use an explicit path",
    )
}

pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let value = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    #[cfg(not(windows))]
    let value = std::env::var_os("HOME");
    value.map(PathBuf::from)
}

fn read_yaml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, AppError> {
    let input = fs::read_to_string(path).map_err(|error| config_error(path, error.to_string()))?;
    serde_yaml::from_str(&input)
        .map_err(|error| config_error(path, format!("invalid YAML at {}: {error}", path.display())))
}

fn config_error(path: &Path, reason: impl Into<String>) -> AppError {
    AppError::new(
        AppErrorCode::InvalidConfig,
        "config.load",
        path.display().to_string(),
        reason,
        "Correct the configuration file and retry",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_config_deterministically_overrides_targets_and_expands_home() {
        let shared = WorkspaceConfig {
            default_channel: "prod".to_owned(),
            targets: BTreeMap::from([
                (
                    "claude".to_owned(),
                    TargetConfig {
                        root: PathBuf::from("shared"),
                    },
                ),
                (
                    "cursor".to_owned(),
                    TargetConfig {
                        root: PathBuf::from("cursor"),
                    },
                ),
            ]),
        };
        let local = LocalWorkspaceConfig {
            default_channel: None,
            targets: BTreeMap::from([(
                "claude".to_owned(),
                TargetConfig {
                    root: PathBuf::from("~/.claude"),
                },
            )]),
        };
        let home = Path::new("home");
        let merged = merge_configs(shared, local, Some(home)).unwrap();
        assert_eq!(merged.default_channel, "prod");
        assert_eq!(merged.targets["claude"].root, home.join(".claude"));
        assert_eq!(merged.targets["cursor"].root, PathBuf::from("cursor"));
    }

    #[test]
    fn saving_a_local_target_preserves_other_local_settings() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join(WORKSPACE_CONFIG_FILE),
            "default_channel: prod\ntargets:\n  trae:\n    root: shared/trae\n",
        )
        .unwrap();
        fs::write(
            temp.path().join(LOCAL_WORKSPACE_CONFIG_FILE),
            "default_channel: staging\ntargets:\n  cursor:\n    root: local/cursor\n",
        )
        .unwrap();

        save_local_target(temp.path(), "workbuddy", PathBuf::from("local/workbuddy")).unwrap();

        let local: LocalWorkspaceConfig =
            read_yaml(&temp.path().join(LOCAL_WORKSPACE_CONFIG_FILE)).unwrap();
        assert_eq!(local.default_channel.as_deref(), Some("staging"));
        assert_eq!(local.targets["cursor"].root, PathBuf::from("local/cursor"));
        assert_eq!(
            local.targets["workbuddy"].root,
            PathBuf::from("local/workbuddy")
        );
        let merged = load_workspace_config(temp.path()).unwrap();
        assert_eq!(merged.default_channel, "staging");
        assert_eq!(merged.targets["trae"].root, PathBuf::from("shared/trae"));
    }

    #[test]
    fn windows_separated_home_path_is_expanded_without_a_hardcoded_root() {
        let home = Path::new("home");
        assert_eq!(
            expand_home(Path::new("~\\tools"), Some(home)).unwrap(),
            home.join("tools")
        );
    }

    #[test]
    fn target_profile_resolves_relative_roots_from_the_asset_workspace() {
        let absolute = std::env::temp_dir().join("absolute-cursor");
        let config = WorkspaceConfig {
            default_channel: "prod".to_owned(),
            targets: BTreeMap::from([
                (
                    "trae".to_owned(),
                    TargetConfig {
                        root: PathBuf::from(".trae"),
                    },
                ),
                (
                    "cursor".to_owned(),
                    TargetConfig {
                        root: absolute.clone(),
                    },
                ),
            ]),
        };
        let workspace = Path::new("workspace");

        assert_eq!(
            config.target_profile(workspace, "trae").unwrap().root,
            workspace.join(".trae")
        );
        assert_eq!(
            config.target_profile(workspace, "cursor").unwrap().root,
            absolute
        );
        assert_eq!(
            config.target_profile(workspace, "codex").unwrap_err().code,
            AppErrorCode::InvalidConfig
        );
    }
}
