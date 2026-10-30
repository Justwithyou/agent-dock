use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use adock_core::{AppError, AppErrorCode, WorkspaceInfo};

use crate::WORKSPACE_CONFIG_FILE;

const PROMPT_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "AgentDock Prompt",
  "type": "object",
  "additionalProperties": false,
  "required": ["apiVersion", "kind", "id", "title", "description", "owners", "spec", "body"],
  "properties": {
    "apiVersion": {"const": "adock/v1alpha1"},
    "kind": {"const": "prompt"},
    "id": {"type": "string", "pattern": "^[a-z0-9]+(?:-[a-z0-9]+)*$"},
    "title": {"type": "string", "minLength": 1},
    "description": {"type": "string", "minLength": 1},
    "owners": {"type": "array", "minItems": 1, "uniqueItems": true, "items": {"type": "string", "minLength": 1}},
    "labels": {"type": "array", "uniqueItems": true, "items": {"type": "string"}},
    "body": {"type": "string", "minLength": 1},
    "spec": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "variables": {
          "type": "array",
          "default": [],
          "items": {
            "type": "object",
            "additionalProperties": false,
            "required": ["name"],
            "properties": {
              "name": {"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$"},
              "description": {"type": "string", "default": ""},
              "required": {"type": "boolean", "default": false},
              "default": {"type": ["string", "null"], "default": null}
            }
          }
        }
      }
    }
  }
}
"#;

const SKILL_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "AgentDock Skill",
  "type": "object",
  "additionalProperties": false,
  "required": ["apiVersion", "kind", "id", "title", "description", "owners", "spec", "body"],
  "properties": {
    "apiVersion": {"const": "adock/v1alpha1"},
    "kind": {"const": "skill"},
    "id": {"type": "string", "pattern": "^[a-z0-9]+(?:-[a-z0-9]+)*$"},
    "title": {"type": "string", "minLength": 1},
    "description": {"type": "string", "minLength": 1},
    "owners": {"type": "array", "minItems": 1, "uniqueItems": true, "items": {"type": "string", "minLength": 1}},
    "labels": {"type": "array", "uniqueItems": true, "items": {"type": "string"}},
    "body": {"type": "string", "minLength": 1},
    "spec": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "triggers": {"type": "array", "default": [], "items": {"type": "string"}},
        "dependencies": {"type": "array", "default": [], "items": {"type": "string"}},
        "entrypoint": {"type": ["string", "null"], "default": null}
      }
    }
  }
}
"#;

const EXAMPLE_PROMPT: &str = r#"---
apiVersion: adock/v1alpha1
kind: prompt
id: code-review
title: Code Review
description: Review a change for correctness and maintainability.
owners: [team]
labels: [engineering]
spec:
  variables:
    - name: change
      description: Change or diff to review
      required: true
---
Review the following change:

{{change}}
"#;

const EXAMPLE_SKILL: &str = r#"---
apiVersion: adock/v1alpha1
kind: skill
id: repo-onboarding
title: Repository Onboarding
description: Learn the structure and conventions of a repository.
owners: [team]
labels: [onboarding]
spec:
  triggers: [new repository]
  dependencies: []
  entrypoint: scripts/collect.sh
---
Read references/start-here.md and inspect the repository before making changes.
"#;

#[derive(Debug, Clone, Copy, Default)]
pub struct WorkspaceInitializer;

impl WorkspaceInitializer {
    pub fn initialize(path: &Path) -> Result<WorkspaceInfo, AppError> {
        match fs::metadata(path) {
            Ok(metadata) if !metadata.is_dir() => {
                return Err(init_error(path, "workspace path is not a directory"));
            }
            Ok(_) => {
                let mut entries =
                    fs::read_dir(path).map_err(|error| init_error(path, error.to_string()))?;
                if entries
                    .next()
                    .transpose()
                    .map_err(|error| init_error(path, error.to_string()))?
                    .is_some()
                {
                    return Err(AppError::new(
                        AppErrorCode::WorkspaceNotEmpty,
                        "workspace.initialize",
                        path.display().to_string(),
                        "refusing to initialize a non-empty directory",
                        "Choose an empty directory; existing files were not changed",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(path).map_err(|error| init_error(path, error.to_string()))?;
            }
            Err(error) => return Err(init_error(path, error.to_string())),
        }

        for directory in [
            "prompts",
            "schemas",
            "skills/repo-onboarding/scripts",
            "skills/repo-onboarding/references",
        ] {
            fs::create_dir_all(path.join(directory))
                .map_err(|error| init_error(path, error.to_string()))?;
        }
        let files = [
            (
                WORKSPACE_CONFIG_FILE,
                "default_channel: prod\ntargets: {}\n",
            ),
            (".gitignore", "adock.local.yaml\n"),
            (
                "README.md",
                "# AgentDock Assets\n\nPrompts live in prompts/. Skills live in skills/<id>/SKILL.md.\n",
            ),
            ("schemas/prompt.schema.json", PROMPT_SCHEMA),
            ("schemas/skill.schema.json", SKILL_SCHEMA),
            ("prompts/code-review.md", EXAMPLE_PROMPT),
            ("skills/repo-onboarding/SKILL.md", EXAMPLE_SKILL),
            (
                "skills/repo-onboarding/scripts/collect.sh",
                "# Add repository-specific read-only collection commands here.\n",
            ),
            (
                "skills/repo-onboarding/references/start-here.md",
                "# Start here\n\nRecord repository conventions in this file.\n",
            ),
        ];
        for (relative, content) in files {
            write_new(&path.join(relative), content)?;
        }
        let root = path
            .canonicalize()
            .map_err(|error| init_error(path, error.to_string()))?;
        Ok(WorkspaceInfo {
            config_path: root.join(WORKSPACE_CONFIG_FILE),
            git_root: find_git_root(&root),
            root,
        })
    }
}

fn write_new(path: &Path, content: &str) -> Result<(), AppError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| init_error(path, error.to_string()))?;
    file.write_all(content.as_bytes())
        .map_err(|error| init_error(path, error.to_string()))
}

fn init_error(path: &Path, reason: impl Into<String>) -> AppError {
    AppError::new(
        AppErrorCode::FileWrite,
        "workspace.initialize",
        path.display().to_string(),
        reason,
        "Check the directory and write permissions, then retry with an empty directory",
    )
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|directory| directory.join(".git").exists())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use adock_core::{Asset, AssetSpec, parse_asset};
    use serde_json::Value;
    use tempfile::TempDir;

    use crate::WorkspaceScanner;

    use super::*;

    #[test]
    fn initializes_complete_examples_and_never_overwrites_non_empty_directories() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("assets");
        let info = WorkspaceInitializer::initialize(&root).unwrap();
        assert_eq!(info.root, root.canonicalize().unwrap());
        let scan = WorkspaceScanner::new(&root).scan().unwrap();
        assert_eq!(scan.assets.len(), 2);
        assert!(scan.issues.is_empty());
        for schema in ["prompt.schema.json", "skill.schema.json"] {
            let value: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(root.join("schemas").join(schema)).unwrap(),
            )
            .unwrap();
            assert_eq!(value["properties"]["apiVersion"]["const"], "adock/v1alpha1");
        }

        let prompt = root.join("prompts/code-review.md");
        let before = fs::read(&prompt).unwrap();
        let error = WorkspaceInitializer::initialize(&root).unwrap_err();
        assert_eq!(error.code, AppErrorCode::WorkspaceNotEmpty);
        assert_eq!(fs::read(&prompt).unwrap(), before);
    }

    #[test]
    fn initialized_examples_satisfy_model_schema_contracts() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("assets");
        WorkspaceInitializer::initialize(&root).unwrap();

        for (asset_path, schema_path) in [
            ("prompts/code-review.md", "schemas/prompt.schema.json"),
            (
                "skills/repo-onboarding/SKILL.md",
                "schemas/skill.schema.json",
            ),
        ] {
            let asset = parse_asset(
                Path::new(asset_path),
                &fs::read_to_string(root.join(asset_path)).unwrap(),
            )
            .unwrap();
            let schema: Value =
                serde_json::from_str(&fs::read_to_string(root.join(schema_path)).unwrap()).unwrap();
            let validator = jsonschema::validator_for(&schema).unwrap();
            let document = model_document(&asset);
            let errors = validator
                .iter_errors(&document)
                .map(|error| error.to_string())
                .collect::<Vec<_>>();
            assert!(errors.is_empty(), "{asset_path}: {errors:?}");
        }
    }

    #[test]
    fn schemas_reject_model_invalid_documents() {
        let prompt = parse_asset(Path::new("prompts/code-review.md"), EXAMPLE_PROMPT).unwrap();
        let prompt_schema: Value = serde_json::from_str(PROMPT_SCHEMA).unwrap();
        let prompt_validator = jsonschema::validator_for(&prompt_schema).unwrap();
        let valid_prompt = model_document(&prompt);

        let mut missing_title = valid_prompt.clone();
        missing_title.as_object_mut().unwrap().remove("title");
        assert!(!prompt_validator.is_valid(&missing_title));

        let mut unknown_field = valid_prompt.clone();
        unknown_field["unexpected"] = Value::Bool(true);
        assert!(!prompt_validator.is_valid(&unknown_field));

        let mut invalid_spec = valid_prompt.clone();
        invalid_spec["spec"]["variables"] = Value::String("not-an-array".to_owned());
        assert!(!prompt_validator.is_valid(&invalid_spec));

        let skill =
            parse_asset(Path::new("skills/repo-onboarding/SKILL.md"), EXAMPLE_SKILL).unwrap();
        let skill_schema: Value = serde_json::from_str(SKILL_SCHEMA).unwrap();
        let skill_validator = jsonschema::validator_for(&skill_schema).unwrap();
        let mut invalid_entrypoint = model_document(&skill);
        invalid_entrypoint["spec"]["entrypoint"] = Value::from(42);
        assert!(!skill_validator.is_valid(&invalid_entrypoint));

        let mut unknown_spec_field = model_document(&skill);
        unknown_spec_field["spec"]["unexpected"] = Value::Bool(true);
        assert!(!skill_validator.is_valid(&unknown_spec_field));
    }

    fn model_document(asset: &Asset) -> Value {
        let mut document = serde_json::to_value(&asset.metadata).unwrap();
        let object = document.as_object_mut().unwrap();
        let spec = match &asset.spec {
            AssetSpec::Prompt(spec) => serde_json::to_value(spec).unwrap(),
            AssetSpec::Skill(spec) => serde_json::to_value(spec).unwrap(),
        };
        object.insert("spec".to_owned(), spec);
        object.insert("body".to_owned(), Value::String(asset.body.clone()));
        document
    }
}
