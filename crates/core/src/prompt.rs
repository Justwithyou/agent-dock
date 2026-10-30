use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{AppError, AppErrorCode, Asset, AssetSpec, validation::valid_variable_name};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptRenderResult {
    pub body: String,
    pub values: BTreeMap<String, String>,
}

pub fn extract_prompt_variables(body: &str) -> Result<Vec<String>, AppError> {
    Ok(placeholders(body)?
        .into_iter()
        .map(|placeholder| placeholder.name)
        .collect())
}

pub fn render_prompt(
    asset: &Asset,
    values: Option<&BTreeMap<String, String>>,
) -> Result<PromptRenderResult, AppError> {
    let AssetSpec::Prompt(spec) = &asset.spec else {
        return Err(AppError::new(
            AppErrorCode::InvalidInput,
            "prompt.render",
            asset.metadata.id.clone(),
            "only Prompt assets can be rendered",
            "Choose a Prompt asset",
        ));
    };
    let Some(explicit) = values else {
        return Ok(PromptRenderResult {
            body: asset.body.clone(),
            values: BTreeMap::new(),
        });
    };

    let reference = asset.reference()?;
    let placeholders = placeholders(&asset.body)?;
    let declared: BTreeSet<&str> = spec
        .variables
        .iter()
        .map(|variable| variable.name.as_str())
        .collect();
    if declared.len() != spec.variables.len() {
        return Err(render_error(
            &reference.to_string(),
            "duplicate variable declarations",
        ));
    }
    if let Some(variable) = spec
        .variables
        .iter()
        .find(|variable| !valid_variable_name(&variable.name))
    {
        return Err(render_error(
            &format!("{}:spec.variables", reference),
            &format!("invalid variable name '{}'", variable.name),
        ));
    }
    if let Some(placeholder) = placeholders
        .iter()
        .find(|placeholder| !declared.contains(placeholder.name.as_str()))
    {
        return Err(render_error(
            &format!("{}:body", reference),
            &format!(
                "placeholder '{}' is not declared in spec.variables",
                placeholder.name
            ),
        ));
    }
    if let Some(name) = explicit
        .keys()
        .find(|name| !declared.contains(name.as_str()))
    {
        return Err(render_error(
            &format!("{}:variables", reference),
            &format!("value supplied for undeclared variable '{name}'"),
        ));
    }

    let mut resolved = BTreeMap::new();
    for variable in &spec.variables {
        let value = explicit
            .get(&variable.name)
            .cloned()
            .or_else(|| variable.default.clone());
        match value {
            Some(value) => {
                resolved.insert(variable.name.clone(), value);
            }
            None if variable.required => {
                return Err(render_error(
                    &format!("{}:spec.variables.{}", reference, variable.name),
                    &format!(
                        "required variable '{}' has no explicit value or default",
                        variable.name
                    ),
                ));
            }
            None => {
                resolved.insert(variable.name.clone(), String::new());
            }
        }
    }

    let mut body = String::with_capacity(asset.body.len());
    let mut cursor = 0;
    for placeholder in placeholders {
        body.push_str(&asset.body[cursor..placeholder.start]);
        body.push_str(
            resolved
                .get(&placeholder.name)
                .expect("all placeholders were checked against declarations"),
        );
        cursor = placeholder.end;
    }
    body.push_str(&asset.body[cursor..]);
    Ok(PromptRenderResult {
        body,
        values: resolved,
    })
}

#[derive(Debug)]
struct Placeholder {
    start: usize,
    end: usize,
    name: String,
}

fn placeholders(body: &str) -> Result<Vec<Placeholder>, AppError> {
    let mut result = Vec::new();
    let mut cursor = 0;
    while let Some(opening) = body[cursor..].find("{{") {
        let start = cursor + opening;
        let name_start = start + 2;
        let Some(closing) = body[name_start..].find("}}") else {
            return Err(AppError::new(
                AppErrorCode::InvalidAsset,
                "prompt.placeholders",
                "body",
                "unclosed {{variable}} placeholder",
                "Close the placeholder with }}",
            ));
        };
        let name_end = name_start + closing;
        let name = body[name_start..name_end].trim();
        if !valid_variable_name(name) {
            return Err(AppError::new(
                AppErrorCode::InvalidAsset,
                "prompt.placeholders",
                name,
                "placeholder names must match [A-Za-z_][A-Za-z0-9_]*",
                "Declare and use a simple variable name",
            ));
        }
        result.push(Placeholder {
            start,
            end: name_end + 2,
            name: name.to_owned(),
        });
        cursor = name_end + 2;
    }
    Ok(result)
}

fn render_error(subject: &str, reason: &str) -> AppError {
    AppError::new(
        AppErrorCode::ValidationFailed,
        "prompt.render",
        subject,
        reason,
        "Declare variables in spec.variables and provide all required values",
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{API_VERSION, AssetKind, AssetMetadata, PromptSpec, PromptVariable};

    use super::*;

    fn prompt() -> Asset {
        Asset {
            metadata: AssetMetadata {
                api_version: API_VERSION.to_owned(),
                kind: AssetKind::Prompt,
                id: "review".to_owned(),
                title: "Review".to_owned(),
                description: "Review code".to_owned(),
                owners: vec!["alice".to_owned()],
                labels: vec![],
            },
            spec: AssetSpec::Prompt(PromptSpec {
                variables: vec![
                    PromptVariable {
                        name: "path".to_owned(),
                        description: String::new(),
                        required: true,
                        default: None,
                    },
                    PromptVariable {
                        name: "style".to_owned(),
                        description: String::new(),
                        required: false,
                        default: Some("concise".to_owned()),
                    },
                ],
            }),
            body: "Review {{path}} in a {{ style }} style".to_owned(),
            source_files: vec![PathBuf::from("prompts/review.md")],
        }
    }

    #[test]
    fn preserves_template_without_explicit_render_request() {
        let rendered = render_prompt(&prompt(), None).unwrap();
        assert_eq!(rendered.body, prompt().body);
    }

    #[test]
    fn explicit_values_override_defaults() {
        let values = BTreeMap::from([
            ("path".to_owned(), "src/lib.rs".to_owned()),
            ("style".to_owned(), "detailed".to_owned()),
        ]);
        let rendered = render_prompt(&prompt(), Some(&values)).unwrap();
        assert_eq!(rendered.body, "Review src/lib.rs in a detailed style");
    }

    #[test]
    fn defaults_apply_and_required_values_are_enforced() {
        let values = BTreeMap::from([("path".to_owned(), "src".to_owned())]);
        assert_eq!(
            render_prompt(&prompt(), Some(&values)).unwrap().body,
            "Review src in a concise style"
        );
        let error = render_prompt(&prompt(), Some(&BTreeMap::new())).unwrap_err();
        assert_eq!(error.code, AppErrorCode::ValidationFailed);
        assert!(error.reason.contains("path"));
    }

    #[test]
    fn rejects_undeclared_placeholders_and_values() {
        let mut asset = prompt();
        asset.body = "{{unknown}}".to_owned();
        assert!(render_prompt(&asset, Some(&BTreeMap::new())).is_err());
        let values = BTreeMap::from([
            ("path".to_owned(), "src".to_owned()),
            ("unknown".to_owned(), "value".to_owned()),
        ]);
        assert!(render_prompt(&prompt(), Some(&values)).is_err());
    }
}
