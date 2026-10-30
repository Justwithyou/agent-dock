use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::ExitCode,
};

use adock_adapters::GitRepository;
use adock_core::{
    AppError, AppErrorCode, Asset, AssetDiff, AssetKind, AssetRef, AssetSpec, ChangeKind,
    DiagnosticSeverity, RevisionSelector, ValidationReport, VersionTag, WorkspaceInfo,
    WorkspaceService, render_prompt,
};
use adock_infra::{
    AssetFilter, AssetListResult, AssetService, FileSystemWorkspaceDiscovery, WorkspaceInitializer,
};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "adock",
    version,
    about = "Manage Prompt and Skill assets and their Git versions",
    subcommand_required = true,
    arg_required_else_help = true,
    after_help = "Revision selectors: working-tree, HEAD, a full commit SHA, or adock/<kind>/<id>/<name>.\nDefaults: show uses working-tree; log and tag create use HEAD; diff and rollback require explicit revisions.\nVersion tags are never pushed automatically. Rollback never commits, pushes, or moves prod."
)]
struct Cli {
    /// Workspace directory or a directory inside it (default: current directory).
    #[arg(long, global = true, value_name = "PATH")]
    workspace: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Initialize an empty directory with example assets and schemas.
    Init,
    /// Discover and print workspace information.
    Workspace {
        /// Optional starting path; defaults to --workspace or the current directory.
        path: Option<PathBuf>,
    },
    /// List assets in the current working tree.
    List {
        /// Filter by asset type: prompt or skill.
        #[arg(long, value_parser = parse_kind)]
        kind: Option<AssetKind>,
        /// Filter by an exact owner.
        #[arg(long)]
        owner: Option<String>,
        /// Filter by an exact classification label.
        #[arg(long)]
        label: Option<String>,
        /// Case-insensitive search in id, title, and description.
        #[arg(long)]
        query: Option<String>,
    },
    /// Show one asset; defaults to working-tree unless --ref is supplied.
    Show {
        /// Asset type: prompt or skill.
        #[arg(value_parser = parse_kind)]
        kind: AssetKind,
        /// Asset identifier.
        id: String,
        /// Read a committed snapshot instead of the default working-tree asset.
        #[arg(long = "ref", value_name = "REV", value_parser = parse_revision)]
        reference: Option<RevisionSelector>,
        /// Explicitly render a Prompt with NAME=VALUE; repeatable and not valid for Skills.
        #[arg(long = "var", value_name = "NAME=VALUE")]
        variables: Vec<String>,
    },
    /// Validate the whole workspace, or one asset when kind and id are supplied.
    Validate {
        /// Asset type: prompt or skill.
        #[arg(value_parser = parse_kind, requires = "id")]
        kind: Option<AssetKind>,
        /// Asset identifier.
        #[arg(requires = "kind")]
        id: Option<String>,
    },
    /// Show commits that changed the asset path, starting from HEAD.
    Log {
        /// Asset type: prompt or skill.
        #[arg(value_parser = parse_kind)]
        kind: AssetKind,
        /// Asset identifier.
        id: String,
        /// Maximum number of commits to print.
        #[arg(long, value_name = "COUNT")]
        limit: Option<usize>,
    },
    /// Compare two explicit asset snapshots; neither revision has a default.
    Diff {
        /// Asset type: prompt or skill.
        #[arg(value_parser = parse_kind)]
        kind: AssetKind,
        /// Asset identifier.
        id: String,
        /// Required source revision.
        #[arg(long, value_name = "REV", value_parser = parse_revision)]
        from: RevisionSelector,
        /// Required destination revision.
        #[arg(long, value_name = "REV", value_parser = parse_revision)]
        to: RevisionSelector,
    },
    /// Create or list asset version tags.
    Tag {
        #[command(subcommand)]
        command: TagCommands,
    },
    /// Preview or restore an asset to --to; without --yes no files are written.
    Rollback {
        /// Asset type: prompt or skill.
        #[arg(value_parser = parse_kind)]
        kind: AssetKind,
        /// Asset identifier.
        id: String,
        /// Required committed target revision: HEAD, a full commit SHA, or an asset tag.
        #[arg(long, value_name = "REV", value_parser = parse_revision)]
        to: RevisionSelector,
        /// Apply the previewed rollback; omit to preview without writing files.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum TagCommands {
    /// Create a tag at HEAD by default; never pushes it.
    #[command(
        long_about = "Create adock/<kind>/<id>/<name> at HEAD by default, or at --ref; never pushes it.\nExisting v* tags cannot be overwritten. prod always requires --move and --yes. Moving any other existing tag also requires --move and --yes."
    )]
    Create {
        /// Asset type: prompt or skill.
        #[arg(value_parser = parse_kind)]
        kind: AssetKind,
        /// Asset identifier.
        id: String,
        /// Version tag name, such as v1.0.0 or prod.
        name: String,
        /// Committed target revision (default: HEAD; working-tree is not allowed).
        #[arg(
            long = "ref",
            value_name = "REV",
            default_value = "HEAD",
            value_parser = parse_revision
        )]
        reference: RevisionSelector,
        /// Explicitly allow a movable tag operation.
        #[arg(long = "move")]
        move_tag: bool,
        /// Confirm prod creation/movement or movement of another existing tag.
        #[arg(long)]
        yes: bool,
    },
    /// List version tags, optionally filtered by kind or by kind and id.
    List {
        /// Asset type: prompt or skill.
        #[arg(value_parser = parse_kind)]
        kind: Option<AssetKind>,
        /// Asset identifier; requires kind.
        #[arg(requires = "kind")]
        id: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = error.exit_code();
            let output = error.to_string();
            if error.use_stderr() {
                eprint!("error[invalid_input]: {output}");
            } else {
                print!("{output}");
            }
            return ExitCode::from(exit_code as u8);
        }
    };

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error[{}]: {error}", error_code(error.code));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), AppError> {
    let workspace = cli.workspace;
    match cli.command {
        Commands::Init => initialize_workspace(workspace.as_deref()),
        Commands::Workspace { path } => {
            let info = discover_workspace(path.as_deref().or(workspace.as_deref()))?;
            print_workspace(&info);
            Ok(())
        }
        Commands::List {
            kind,
            owner,
            label,
            query,
        } => {
            let root = discover_workspace(workspace.as_deref())?.root;
            let result = AssetService::new(root).list(&AssetFilter {
                kind,
                owner,
                label,
                query,
            })?;
            print_asset_list(&result);
            Ok(())
        }
        Commands::Show {
            kind,
            id,
            reference: selector,
            variables,
        } => {
            if kind == AssetKind::Skill && !variables.is_empty() {
                return Err(AppError::new(
                    AppErrorCode::InvalidInput,
                    "cli.show",
                    "--var",
                    "Prompt variables cannot be supplied for a Skill",
                    "Remove --var or choose a Prompt asset",
                ));
            }
            let reference = AssetRef::new(kind, id)?;
            let root = discover_workspace(workspace.as_deref())?.root;
            let (asset, revision, commit) = match selector {
                Some(selector) => {
                    let snapshot = GitRepository::open(&root)?.snapshot(&reference, &selector)?;
                    (snapshot.asset, snapshot.revision, snapshot.commit)
                }
                None => (
                    AssetService::new(&root).show(&reference)?,
                    RevisionSelector::WorkingTree,
                    None,
                ),
            };
            let body = if variables.is_empty() {
                asset.body.clone()
            } else {
                let values = parse_variables(&variables)?;
                render_prompt(&asset, Some(&values))?.body
            };
            print_asset(&asset, &body, &revision, commit.as_deref());
            Ok(())
        }
        Commands::Validate { kind, id } => {
            let reference = optional_asset_ref(kind, id)?;
            let root = discover_workspace(workspace.as_deref())?.root;
            let report = AssetService::new(root).validate(reference.as_ref())?;
            print_validation(&report);
            if report.valid {
                Ok(())
            } else {
                Err(AppError::new(
                    AppErrorCode::ValidationFailed,
                    "cli.validate",
                    reference
                        .map(|reference| reference.to_string())
                        .unwrap_or_else(|| "workspace".to_owned()),
                    format!(
                        "validation found {} error(s) and {} warning(s)",
                        report.error_count, report.warning_count
                    ),
                    "Fix the reported diagnostics and run validate again",
                ))
            }
        }
        Commands::Log { kind, id, limit } => {
            let reference = AssetRef::new(kind, id)?;
            let root = discover_workspace(workspace.as_deref())?.root;
            let entries = GitRepository::open(root)?.log(&reference, limit)?;
            if entries.is_empty() {
                println!("No history found for {reference}.");
            }
            for entry in entries {
                println!(
                    "{} | {} | {}",
                    entry.commit, entry.committed_at, entry.author
                );
                for line in entry.message.lines() {
                    println!("  {line}");
                }
            }
            Ok(())
        }
        Commands::Diff { kind, id, from, to } => {
            let reference = AssetRef::new(kind, id)?;
            let root = discover_workspace(workspace.as_deref())?.root;
            let diff = GitRepository::open(root)?.diff(&reference, &from, &to)?;
            print_diff(&diff);
            Ok(())
        }
        Commands::Tag { command } => match command {
            TagCommands::Create {
                kind,
                id,
                name,
                reference: target,
                move_tag,
                yes,
            } => {
                let tag = VersionTag::new(AssetRef::new(kind, id)?, name)?;
                let root = discover_workspace(workspace.as_deref())?.root;
                let created =
                    GitRepository::open(root)?.create_tag(&tag, &target, move_tag, yes)?;
                println!("Version tag {created} now points to {target}; no push performed.");
                Ok(())
            }
            TagCommands::List { kind, id } => {
                let reference = id
                    .map(|id| AssetRef::new(kind.expect("clap requires kind with id"), id))
                    .transpose()?;
                let root = discover_workspace(workspace.as_deref())?.root;
                let mut tags = GitRepository::open(root)?.list_tags(reference.as_ref())?;
                if reference.is_none() {
                    if let Some(kind) = kind {
                        kind.ensure_supported()?;
                        tags.retain(|tag| tag.asset.kind == kind);
                    }
                }
                if tags.is_empty() {
                    println!("No version tags found.");
                }
                for tag in tags {
                    println!("{tag}");
                }
                Ok(())
            }
        },
        Commands::Rollback { kind, id, to, yes } => {
            let reference = AssetRef::new(kind, id)?;
            let root = discover_workspace(workspace.as_deref())?.root;
            let repository = GitRepository::open(root)?;
            if yes {
                let diff = repository.rollback(&reference, &to, true)?;
                println!(
                    "Rollback applied to working-tree files; no commit, push, or prod move performed."
                );
                print_diff(&diff);
            } else {
                let diff = repository.rollback_preview(&reference, &to)?;
                println!("Rollback preview only; no files changed. Re-run with --yes to apply.");
                print_diff(&diff);
            }
            Ok(())
        }
    }
}

fn initialize_workspace(path: Option<&Path>) -> Result<(), AppError> {
    let path = match path {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir().map_err(|error| {
            AppError::new(
                AppErrorCode::WorkspaceIo,
                "workspace.current_dir",
                "current directory",
                error.to_string(),
                "Pass an explicit workspace path with --workspace",
            )
        })?,
    };
    let info = WorkspaceInitializer::initialize(&path)?;
    println!("Initialized workspace: {}", info.root.display());
    println!("Config: {}", info.config_path.display());
    Ok(())
}

fn discover_workspace(path: Option<&Path>) -> Result<WorkspaceInfo, AppError> {
    WorkspaceService::new(FileSystemWorkspaceDiscovery).discover(path)
}

fn print_workspace(info: &WorkspaceInfo) {
    println!("Workspace: {}", info.root.display());
    println!("Config: {}", info.config_path.display());
}

fn print_asset_list(result: &AssetListResult) {
    if result.assets.is_empty() {
        println!("No assets found.");
    }
    for item in &result.assets {
        println!(
            "{} | {} | {}",
            item.asset,
            item.title,
            item.source_path.display()
        );
        println!("  {}", item.description);
        println!("  owners: {}", item.owners.join(", "));
        if !item.labels.is_empty() {
            println!("  labels: {}", item.labels.join(", "));
        }
    }
    for issue in &result.issues {
        println!(
            "invalid[{}] {}: {}",
            error_code(issue.error.code),
            issue.path.display(),
            issue.error.reason
        );
    }
}

fn print_asset(asset: &Asset, body: &str, revision: &RevisionSelector, commit: Option<&str>) {
    println!(
        "Asset: {}",
        asset.reference().expect("services return valid assets")
    );
    println!("Title: {}", asset.metadata.title);
    println!("Description: {}", asset.metadata.description);
    println!("Owners: {}", asset.metadata.owners.join(", "));
    if !asset.metadata.labels.is_empty() {
        println!("Labels: {}", asset.metadata.labels.join(", "));
    }
    println!("Revision: {revision}");
    if let Some(commit) = commit {
        println!("Commit: {commit}");
    }
    match &asset.spec {
        AssetSpec::Prompt(spec) => {
            println!("Prompt variables:");
            if spec.variables.is_empty() {
                println!("  (none)");
            }
            for variable in &spec.variables {
                let requirement = if variable.required {
                    "required"
                } else {
                    "optional"
                };
                match &variable.default {
                    Some(default) => {
                        println!("  {} ({requirement}, default: {default:?})", variable.name)
                    }
                    None => println!("  {} ({requirement})", variable.name),
                }
            }
        }
        AssetSpec::Skill(spec) => {
            println!("Triggers: {}", display_list(&spec.triggers));
            println!("Dependencies: {}", display_list(&spec.dependencies));
            if let Some(entrypoint) = &spec.entrypoint {
                println!("Entrypoint: {}", entrypoint.display());
            }
        }
    }
    println!("Source files:");
    for path in &asset.source_files {
        println!("  {}", path.display());
    }
    println!("Body:");
    print!("{body}");
    if !body.ends_with('\n') {
        println!();
    }
}

fn display_list(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_owned()
    } else {
        values.join(", ")
    }
}

fn print_validation(report: &ValidationReport) {
    for diagnostic in &report.diagnostics {
        let severity = match diagnostic.severity {
            DiagnosticSeverity::Error => "error",
            DiagnosticSeverity::Warning => "warning",
        };
        print!(
            "{severity}[{}] {}",
            error_code(diagnostic.code),
            diagnostic.path.display()
        );
        if let Some(asset) = &diagnostic.asset {
            print!(" ({asset})");
        }
        if let Some(field) = &diagnostic.field {
            print!(" [{field}]");
        }
        println!(": {}", diagnostic.message);
    }
    println!(
        "Validation: {} | assets: {} | errors: {} | warnings: {}",
        if report.valid { "valid" } else { "invalid" },
        report.asset_count,
        report.error_count,
        report.warning_count
    );
}

fn print_diff(diff: &AssetDiff) {
    println!("Asset: {}", diff.asset);
    println!("From: {}", diff.from);
    println!("To: {}", diff.to);
    if let Some(commit) = &diff.from_commit {
        println!("From commit: {commit}");
    }
    if let Some(commit) = &diff.to_commit {
        println!("To commit: {commit}");
    }
    if diff.is_empty() {
        println!("No changes.");
    }
    for change in &diff.files {
        let kind = match change.change {
            ChangeKind::Added => "added",
            ChangeKind::Removed => "removed",
            ChangeKind::Modified => "modified",
        };
        println!("{kind}: {}", change.path.display());
        if change.binary {
            println!("Binary files differ");
        } else if let Some(text) = &change.text_diff {
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
        }
    }
}

fn optional_asset_ref(
    kind: Option<AssetKind>,
    id: Option<String>,
) -> Result<Option<AssetRef>, AppError> {
    match (kind, id) {
        (Some(kind), Some(id)) => AssetRef::new(kind, id).map(Some),
        (None, None) => Ok(None),
        _ => Err(AppError::new(
            AppErrorCode::InvalidInput,
            "cli.asset_ref",
            "kind/id",
            "kind and id must be supplied together",
            "Supply both arguments or neither",
        )),
    }
}

fn parse_variables(values: &[String]) -> Result<BTreeMap<String, String>, AppError> {
    values
        .iter()
        .map(|value| {
            let (name, value) = value.split_once('=').ok_or_else(|| {
                AppError::new(
                    AppErrorCode::InvalidInput,
                    "cli.var.parse",
                    value,
                    "expected NAME=VALUE",
                    "Repeat --var with a variable name and value, for example --var change=PATCH",
                )
            })?;
            Ok((name.to_owned(), value.to_owned()))
        })
        .collect()
}

fn parse_kind(value: &str) -> Result<AssetKind, AppError> {
    value.parse()
}

fn parse_revision(value: &str) -> Result<RevisionSelector, AppError> {
    value.parse()
}

fn error_code(code: AppErrorCode) -> &'static str {
    match code {
        AppErrorCode::InvalidInput => "invalid_input",
        AppErrorCode::InvalidAssetRef => "invalid_asset_ref",
        AppErrorCode::ReservedAssetKind => "reserved_asset_kind",
        AppErrorCode::InvalidMetadata => "invalid_metadata",
        AppErrorCode::InvalidAsset => "invalid_asset",
        AppErrorCode::InvalidConfig => "invalid_config",
        AppErrorCode::ValidationFailed => "validation_failed",
        AppErrorCode::InvalidRevision => "invalid_revision",
        AppErrorCode::UnknownRevision => "unknown_revision",
        AppErrorCode::WorkspaceNotFound => "workspace_not_found",
        AppErrorCode::WorkspaceNotEmpty => "workspace_not_empty",
        AppErrorCode::NotGitRepository => "not_git_repository",
        AppErrorCode::WorkspaceIo => "workspace_io",
        AppErrorCode::FileWrite => "file_write",
        AppErrorCode::AssetNotFound => "asset_not_found",
        AppErrorCode::DuplicateAsset => "duplicate_asset",
        AppErrorCode::GitUnavailable => "git_unavailable",
        AppErrorCode::GitCommandFailed => "git_command_failed",
        AppErrorCode::TagAlreadyExists => "tag_already_exists",
        AppErrorCode::ConfirmationRequired => "confirmation_required",
        AppErrorCode::DirtyAsset => "dirty_asset",
        AppErrorCode::UnsupportedAdapter => "unsupported_adapter",
        AppErrorCode::DistributionConflict => "distribution_conflict",
    }
}
