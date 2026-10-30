use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{
    AppError, AppErrorCode, Asset, AssetKind, AssetMetadata, AssetSpec, PromptSpec, SkillSpec,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAsset {
    #[serde(rename = "apiVersion")]
    api_version: String,
    kind: AssetKind,
    id: String,
    title: String,
    description: String,
    owners: Vec<String>,
    #[serde(default)]
    labels: Vec<String>,
    spec: serde_yaml::Value,
}

pub fn parse_asset(source_path: &Path, input: &str) -> Result<Asset, AppError> {
    parse_asset_with_sources(source_path, input, vec![source_path.to_path_buf()])
}

pub fn parse_asset_with_sources(
    source_path: &Path,
    input: &str,
    source_files: Vec<PathBuf>,
) -> Result<Asset, AppError> {
    let (front_matter, body) = split_front_matter(source_path, input)?;
    let raw: RawAsset = serde_yaml::from_str(front_matter).map_err(|error| {
        parse_error(
            source_path,
            "front_matter",
            format!("invalid YAML: {error}"),
        )
    })?;
    let metadata = AssetMetadata {
        api_version: raw.api_version,
        kind: raw.kind,
        id: raw.id,
        title: raw.title,
        description: raw.description,
        owners: raw.owners,
        labels: raw.labels,
    };
    metadata.validate().map_err(|error| {
        AppError::new(
            error.code,
            "asset.parse",
            source_path.display().to_string(),
            error.reason,
            error.suggestion,
        )
    })?;

    let spec = match metadata.kind {
        AssetKind::Prompt => AssetSpec::Prompt(
            serde_yaml::from_value::<PromptSpec>(raw.spec).map_err(|error| {
                parse_error(source_path, "spec", format!("invalid Prompt spec: {error}"))
            })?,
        ),
        AssetKind::Skill => AssetSpec::Skill(
            serde_yaml::from_value::<SkillSpec>(raw.spec).map_err(|error| {
                parse_error(source_path, "spec", format!("invalid Skill spec: {error}"))
            })?,
        ),
        AssetKind::McpReserved => {
            return Err(AppError::new(
                AppErrorCode::ReservedAssetKind,
                "asset.parse",
                source_path.display().to_string(),
                "MCP is a reserved asset kind and is outside the current MVP",
                "Use prompt or skill instead",
            ));
        }
    };

    if body.trim().is_empty() {
        return Err(parse_error(
            source_path,
            "body",
            "Markdown body is required",
        ));
    }

    Ok(Asset {
        metadata,
        spec,
        body: body.to_owned(),
        source_files,
    })
}

fn split_front_matter<'a>(
    source_path: &Path,
    input: &'a str,
) -> Result<(&'a str, &'a str), AppError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut offset = 0;
    let mut lines = input.split_inclusive('\n');
    let Some(opening) = lines.next() else {
        return Err(parse_error(
            source_path,
            "front_matter",
            "expected YAML Front Matter delimited by ---",
        ));
    };
    offset += opening.len();
    if opening.trim_end_matches(['\r', '\n']) != "---" {
        return Err(parse_error(
            source_path,
            "front_matter",
            "the first line must be ---",
        ));
    }
    let front_matter_start = offset;
    let mut front_matter_end = None;
    let mut body_start = None;
    for line in lines {
        offset += line.len();
        if line.trim_end_matches(['\r', '\n']) == "---" {
            front_matter_end = Some(offset - line.len());
            body_start = Some(offset);
            break;
        }
    }
    match (front_matter_end, body_start) {
        (Some(end), Some(body)) => Ok((&input[front_matter_start..end], &input[body..])),
        _ => Err(parse_error(
            source_path,
            "front_matter",
            "missing closing --- delimiter",
        )),
    }
}

fn parse_error(path: &Path, field: &str, reason: impl Into<String>) -> AppError {
    AppError::new(
        AppErrorCode::InvalidAsset,
        "asset.parse",
        format!("{}:{field}", path.display()),
        reason,
        "Correct the asset file and run validation again",
    )
}

#[cfg(test)]
mod tests {
    use crate::API_VERSION;

    use super::*;

    #[test]
    fn parses_crlf_front_matter_and_markdown() {
        let input = "---\r\napiVersion: adock/v1alpha1\r\nkind: prompt\r\nid: review\r\ntitle: Review\r\ndescription: Review code\r\nowners: [alice]\r\nspec:\r\n  variables:\r\n    - name: path\r\n      required: true\r\n---\r\nReview {{path}}\r\n";
        let asset = parse_asset(Path::new("prompts/review.md"), input).unwrap();
        assert_eq!(asset.body, "Review {{path}}\r\n");
        assert_eq!(asset.metadata.api_version, API_VERSION);
    }

    #[test]
    fn yaml_error_contains_path_and_field() {
        let error =
            parse_asset(Path::new("prompts/bad.md"), "---\nkind: [\n---\nbody\n").unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidAsset);
        assert!(error.subject.contains("prompts/bad.md"));
        assert!(error.subject.contains("front_matter"));
    }
}
