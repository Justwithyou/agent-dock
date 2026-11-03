use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

const UNKNOWN_REVISION: &str = "0000000000000000000000000000000000000000";

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    first: String,
    second: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();
        initialize_repository(&root);
        write_prompt(&root, "first prompt");
        write_skill(&root, "first skill");
        fs::write(root.join("skills/onboard/obsolete.txt"), "remove me\n").unwrap();
        commit_all(&root, "first assets");
        let first = head(&root);

        write_prompt(&root, "second prompt");
        write_skill(&root, "second skill");
        fs::remove_file(root.join("skills/onboard/obsolete.txt")).unwrap();
        fs::create_dir_all(root.join("skills/onboard/references")).unwrap();
        fs::write(
            root.join("skills/onboard/references/guide.md"),
            "second guide\n",
        )
        .unwrap();
        commit_all(&root, "second assets");
        let second = head(&root);

        Self {
            _temp: temp,
            root,
            first,
            second,
        }
    }
}

fn adock(root: &Path) -> Command {
    let mut command = Command::cargo_bin("adock").unwrap();
    command.arg("--workspace").arg(root);
    command
}

fn initialize_repository(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "AgentDock CLI Tests"]);
    git(root, &["config", "user.email", "tests@example.invalid"]);
    fs::write(
        root.join("adock.yaml"),
        "default_channel: prod\ntargets:\n  trae:\n    root: target/trae\n  codex:\n    root: target/codex\n",
    )
    .unwrap();
}

fn write_prompt(root: &Path, marker: &str) {
    fs::create_dir_all(root.join("prompts")).unwrap();
    fs::write(
        root.join("prompts/review.md"),
        format!(
            "---\napiVersion: adock/v1alpha1\nkind: prompt\nid: review\ntitle: Review\ndescription: Review code for correctness\nowners: [alice]\nlabels: [engineering]\nspec:\n  variables:\n    - name: change\n      required: true\n    - name: tone\n      default: concise\n---\nReview {{{{change}}}} in a {{{{tone}}}} style.\n{marker}\n"
        ),
    )
    .unwrap();
}

fn write_skill(root: &Path, marker: &str) {
    fs::create_dir_all(root.join("skills/onboard/scripts")).unwrap();
    fs::write(
        root.join("skills/onboard/SKILL.md"),
        format!(
            "---\napiVersion: adock/v1alpha1\nkind: skill\nid: onboard\ntitle: Onboard\ndescription: Onboard a repository\nowners: [bob]\nlabels: [onboarding]\nspec:\n  triggers: [new repository]\n  dependencies: []\n  entrypoint: scripts/run.sh\n---\n{marker}\n"
        ),
    )
    .unwrap();
    fs::write(
        root.join("skills/onboard/scripts/run.sh"),
        format!("echo {marker}\n"),
    )
    .unwrap();
}

fn commit_all(root: &Path, message: &str) {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", message]);
}

fn git(root: &Path, args: &[&str]) {
    let output = ProcessCommand::new("git")
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
    let output = ProcessCommand::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn native_path(path: &str) -> String {
    path.split('/').collect::<PathBuf>().display().to_string()
}

#[test]
fn init_creates_assets_and_workspace_discovery_is_preserved() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("assets");

    adock(&root)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized workspace:"));
    assert!(root.join("prompts/code-review.md").is_file());
    assert!(root.join("skills/repo-onboarding/SKILL.md").is_file());

    adock(&root)
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[workspace_not_empty]"));

    git(&root, &["init", "-q"]);
    let mut command = Command::cargo_bin("adock").unwrap();
    command
        .arg("workspace")
        .arg(&root)
        .assert()
        .success()
        .stdout(predicate::str::contains("Workspace:"))
        .stdout(predicate::str::contains("adock.yaml"));
}

#[test]
fn list_filters_and_show_supports_working_tree_history_and_prompt_variables() {
    let fixture = Fixture::new();

    adock(&fixture.root)
        .args([
            "list",
            "--kind",
            "prompt",
            "--owner",
            "alice",
            "--label",
            "engineering",
            "--query",
            "correctness",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("prompt:review"))
        .stdout(predicate::str::contains("skill:onboard").not());

    adock(&fixture.root)
        .args(["list", "--kind", "skill"])
        .assert()
        .success()
        .stdout(predicate::str::contains("skill:onboard"))
        .stdout(predicate::str::contains("prompt:review").not());

    write_prompt(&fixture.root, "working prompt");
    adock(&fixture.root)
        .args(["show", "prompt", "review"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Revision: working-tree"))
        .stdout(predicate::str::contains("working prompt"))
        .stdout(predicate::str::contains("{{change}}"));

    adock(&fixture.root)
        .args(["show", "prompt", "review", "--ref", "HEAD"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Commit: {}",
            fixture.second
        )))
        .stdout(predicate::str::contains("second prompt"))
        .stdout(predicate::str::contains("working prompt").not());

    adock(&fixture.root)
        .args([
            "show",
            "prompt",
            "review",
            "--var",
            "change=PATCH",
            "--var",
            "tone=detailed",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Review PATCH in a detailed style.",
        ))
        .stdout(predicate::str::contains("{{change}}").not());
}

#[test]
fn show_lists_skill_files_and_rejects_prompt_variables_for_skills() {
    let fixture = Fixture::new();

    adock(&fixture.root)
        .args(["show", "skill", "onboard"])
        .assert()
        .success()
        .stdout(predicate::str::contains("second skill"))
        .stdout(predicate::str::contains("Entrypoint:"))
        .stdout(predicate::str::contains("skills/onboard/scripts/run.sh"))
        .stdout(predicate::str::contains(
            "skills/onboard/references/guide.md",
        ));

    adock(&fixture.root)
        .args(["show", "skill", "onboard", "--var", "change=PATCH"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[invalid_input]"))
        .stderr(predicate::str::contains("cannot be supplied for a Skill"));
}

#[test]
fn validate_reports_invalid_assets_and_unknown_ids_with_stable_errors() {
    let fixture = Fixture::new();

    adock(&fixture.root)
        .arg("validate")
        .assert()
        .success()
        .stdout(predicate::str::contains("Validation: valid"));
    adock(&fixture.root)
        .args(["validate", "skill", "onboard"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Validation: valid"));

    adock(&fixture.root)
        .args(["show", "prompt", "missing"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[asset_not_found]"));

    fs::write(fixture.root.join("prompts/review.md"), "broken asset\n").unwrap();
    adock(&fixture.root)
        .arg("validate")
        .assert()
        .failure()
        .stdout(predicate::str::contains("Validation: invalid"))
        .stderr(predicate::str::contains("error[validation_failed]"));
    adock(&fixture.root)
        .args(["validate", "prompt", "review"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("Validation: invalid"))
        .stderr(predicate::str::contains("error[validation_failed]"));
    adock(&fixture.root)
        .args(["show", "prompt", "review"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[invalid_asset]"));
}

#[test]
fn log_and_diff_cover_prompt_and_multifile_skill_history() {
    let fixture = Fixture::new();

    adock(&fixture.root)
        .args(["log", "prompt", "review", "--limit", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&fixture.second))
        .stdout(predicate::str::contains("second assets"))
        .stdout(predicate::str::contains("first assets").not());
    adock(&fixture.root)
        .args(["log", "skill", "onboard", "--limit", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("second assets"));

    adock(&fixture.root)
        .args([
            "diff",
            "prompt",
            "review",
            "--from",
            &fixture.first,
            "--to",
            "HEAD",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("-first prompt"))
        .stdout(predicate::str::contains("+second prompt"));

    adock(&fixture.root)
        .args([
            "diff",
            "skill",
            "onboard",
            "--from",
            &fixture.first,
            "--to",
            &fixture.second,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "modified: {}",
            native_path("skills/onboard/scripts/run.sh")
        )))
        .stdout(predicate::str::contains(format!(
            "removed: {}",
            native_path("skills/onboard/obsolete.txt")
        )))
        .stdout(predicate::str::contains(format!(
            "added: {}",
            native_path("skills/onboard/references/guide.md")
        )));
}

#[test]
fn usage_unknown_revisions_and_unknown_assets_fail_without_fake_success() {
    let fixture = Fixture::new();

    adock(&fixture.root)
        .args(["diff", "prompt", "review", "--from", "HEAD"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[invalid_input]"))
        .stderr(predicate::str::contains("--to"));
    adock(&fixture.root)
        .args(["rollback", "prompt", "review"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[invalid_input]"))
        .stderr(predicate::str::contains("--to"));

    adock(&fixture.root)
        .args(["show", "prompt", "review", "--ref", UNKNOWN_REVISION])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[unknown_revision]"));
    adock(&fixture.root)
        .args([
            "diff", "prompt", "missing", "--from", "HEAD", "--to", "HEAD",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[asset_not_found]"));
}

#[test]
fn tag_create_list_and_confirmation_constraints_use_core_rules() {
    let fixture = Fixture::new();

    adock(&fixture.root)
        .args(["tag", "create", "prompt", "review", "v1.0.0"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "adock/prompt/review/v1.0.0 now points to HEAD",
        ));
    adock(&fixture.root)
        .args([
            "show",
            "prompt",
            "review",
            "--ref",
            "adock/prompt/review/v1.0.0",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("second prompt"));

    adock(&fixture.root)
        .args([
            "tag",
            "create",
            "prompt",
            "review",
            "v1.0.0",
            "--ref",
            &fixture.first,
            "--move",
            "--yes",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[tag_already_exists]"));

    adock(&fixture.root)
        .args([
            "tag",
            "create",
            "prompt",
            "review",
            "candidate",
            "--ref",
            &fixture.first,
        ])
        .assert()
        .success();
    adock(&fixture.root)
        .args(["tag", "create", "prompt", "review", "candidate", "--move"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[confirmation_required]"));
    adock(&fixture.root)
        .args([
            "tag",
            "create",
            "prompt",
            "review",
            "candidate",
            "--move",
            "--yes",
        ])
        .assert()
        .success();

    adock(&fixture.root)
        .args(["tag", "create", "prompt", "review", "prod"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[confirmation_required]"));
    adock(&fixture.root)
        .args([
            "tag", "create", "prompt", "review", "prod", "--move", "--yes",
        ])
        .assert()
        .success();

    adock(&fixture.root)
        .args(["tag", "list", "prompt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("adock/prompt/review/v1.0.0"))
        .stdout(predicate::str::contains("adock/prompt/review/candidate"))
        .stdout(predicate::str::contains("adock/prompt/review/prod"));
    adock(&fixture.root)
        .args(["tag", "list", "skill", "onboard"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No version tags found."));
}

#[test]
fn rollback_preview_does_not_write_and_confirmation_restores_only_target_asset() {
    let fixture = Fixture::new();
    let before_head = head(&fixture.root);

    adock(&fixture.root)
        .args(["rollback", "skill", "onboard", "--to", &fixture.first])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Rollback preview only; no files changed.",
        ));
    assert!(
        fs::read_to_string(fixture.root.join("skills/onboard/SKILL.md"))
            .unwrap()
            .contains("second skill")
    );
    assert!(
        fixture
            .root
            .join("skills/onboard/references/guide.md")
            .exists()
    );
    assert!(!fixture.root.join("skills/onboard/obsolete.txt").exists());

    adock(&fixture.root)
        .args([
            "rollback",
            "skill",
            "onboard",
            "--to",
            &fixture.first,
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rollback applied"));
    assert!(
        fs::read_to_string(fixture.root.join("skills/onboard/SKILL.md"))
            .unwrap()
            .contains("first skill")
    );
    assert!(
        !fixture
            .root
            .join("skills/onboard/references/guide.md")
            .exists()
    );
    assert!(fixture.root.join("skills/onboard/obsolete.txt").exists());
    assert_eq!(head(&fixture.root), before_head);

    write_prompt(&fixture.root, "local prompt");
    for extra in [&[][..], &["--yes"][..]] {
        adock(&fixture.root)
            .args(["rollback", "prompt", "review", "--to", &fixture.first])
            .args(extra)
            .assert()
            .failure()
            .stderr(predicate::str::contains("error[dirty_asset]"));
        assert!(
            fs::read_to_string(fixture.root.join("prompts/review.md"))
                .unwrap()
                .contains("local prompt")
        );
    }
    assert_eq!(head(&fixture.root), before_head);
}

#[test]
fn empty_repository_has_empty_log_but_no_phantom_head_or_tag() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("empty-repo");
    fs::create_dir(&root).unwrap();
    initialize_repository(&root);
    write_prompt(&root, "unborn prompt");

    adock(&root)
        .args(["show", "prompt", "review"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unborn prompt"));
    adock(&root)
        .args(["log", "prompt", "review"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No history found"));
    adock(&root)
        .args(["tag", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No version tags found"));
    adock(&root)
        .args(["show", "prompt", "review", "--ref", "HEAD"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[unknown_revision]"));
    adock(&root)
        .args(["tag", "create", "prompt", "review", "v1.0.0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[unknown_revision]"));
}

#[test]
fn unavailable_git_is_reported_as_a_coded_cli_error() {
    let fixture = Fixture::new();
    let empty_path = TempDir::new().unwrap();

    adock(&fixture.root)
        .env("PATH", empty_path.path())
        .args(["log", "prompt", "review"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[git_unavailable]"));
}

#[test]
fn help_documents_revision_defaults_and_confirmation_rules() {
    Command::cargo_bin("adock")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Revision selectors:"))
        .stdout(predicate::str::contains("diff and rollback require"));
    Command::cargo_bin("adock")
        .unwrap()
        .args(["show", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("working-tree"))
        .stdout(predicate::str::contains("--ref"));
    Command::cargo_bin("adock")
        .unwrap()
        .args(["log", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("HEAD"));
    Command::cargo_bin("adock")
        .unwrap()
        .args(["diff", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--from <REV>"))
        .stdout(predicate::str::contains("--to <REV>"));
    Command::cargo_bin("adock")
        .unwrap()
        .args(["tag", "create", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default: HEAD"))
        .stdout(predicate::str::contains("v*"))
        .stdout(predicate::str::contains("prod"))
        .stdout(predicate::str::contains("--move"))
        .stdout(predicate::str::contains("--yes"));
    Command::cargo_bin("adock")
        .unwrap()
        .args(["rollback", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "without --yes no files are written",
        ))
        .stdout(predicate::str::contains("--to <REV>"));
    Command::cargo_bin("adock")
        .unwrap()
        .args(["distribute", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("workspace prod release channel"))
        .stdout(predicate::str::contains("--target"))
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--overwrite-managed"))
        .stdout(predicate::str::contains("--yes"));
}

#[test]
fn distribute_previews_and_executes_prompt_and_skill_from_traceable_revisions() {
    let fixture = Fixture::new();
    git(
        &fixture.root,
        &["tag", "adock/prompt/review/prod", &fixture.first],
    );
    git(
        &fixture.root,
        &["tag", "adock/skill/onboard/prod", &fixture.second],
    );
    let prompt_path = fixture.root.join("target/trae/.trae/commands/review.md");

    adock(&fixture.root)
        .args([
            "distribute",
            "prompt",
            "review",
            "--target",
            "trae",
            "--var",
            "change=PATCH",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Source: adock/prompt/review/prod"))
        .stdout(predicate::str::contains(format!(
            "Commit: {}",
            fixture.first
        )))
        .stdout(predicate::str::contains("added |"))
        .stdout(predicate::str::contains("Dry-run only"));
    assert!(!prompt_path.exists());

    adock(&fixture.root)
        .args([
            "distribute",
            "prompt",
            "review",
            "--target",
            "trae",
            "--var",
            "change=PATCH",
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Execution status: completed"))
        .stdout(predicate::str::contains("written |"))
        .stdout(predicate::str::contains(format!(
            "Distribution completed from commit {}",
            fixture.first
        )));
    let distributed = fs::read_to_string(&prompt_path).unwrap();
    assert!(distributed.contains("Review PATCH in a concise style."));
    assert!(distributed.contains("first prompt"));
    assert!(!distributed.contains("second prompt"));

    adock(&fixture.root)
        .args([
            "distribute",
            "prompt",
            "review",
            "--target",
            "trae",
            "--var",
            "change=PATCH",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("unchanged |"));
    adock(&fixture.root)
        .args([
            "distribute",
            "prompt",
            "review",
            "--target",
            "trae",
            "--ref",
            "HEAD",
            "--var",
            "change=PATCH",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Source: HEAD"))
        .stdout(predicate::str::contains(format!(
            "Commit: {}",
            fixture.second
        )))
        .stdout(predicate::str::contains("updated |"));

    let skill_entry = fixture
        .root
        .join("target/trae/.trae/skills/onboard/SKILL.md");
    let skill_script = fixture
        .root
        .join("target/trae/.trae/skills/onboard/scripts/run.sh");
    adock(&fixture.root)
        .args([
            "distribute",
            "skill",
            "onboard",
            "--target",
            "trae",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("skill:onboard -> trae"))
        .stdout(predicate::str::contains("added |"));
    assert!(!skill_entry.exists());
    adock(&fixture.root)
        .args([
            "distribute",
            "skill",
            "onboard",
            "--target",
            "trae",
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Execution status: completed"));
    assert!(
        fs::read_to_string(skill_entry)
            .unwrap()
            .contains("second skill")
    );
    assert_eq!(
        fs::read_to_string(skill_script).unwrap(),
        "echo second skill\n"
    );
    adock(&fixture.root)
        .args([
            "distribute",
            "skill",
            "onboard",
            "--target",
            "trae",
            "--var",
            "change=PATCH",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[invalid_input]"));
}

#[test]
fn distribute_blocks_unmanaged_files_and_requires_explicit_managed_overwrite() {
    let fixture = Fixture::new();
    let prompt_path = fixture.root.join("target/trae/.trae/commands/review.md");
    fs::create_dir_all(prompt_path.parent().unwrap()).unwrap();
    fs::write(&prompt_path, "unmanaged local file\n").unwrap();

    adock(&fixture.root)
        .args([
            "distribute",
            "prompt",
            "review",
            "--target",
            "trae",
            "--ref",
            "HEAD",
            "--var",
            "change=PATCH",
            "--overwrite-managed",
            "--yes",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("Plan status: blocked"))
        .stdout(predicate::str::contains("unmanaged_file"))
        .stderr(predicate::str::contains("error[distribution_conflict]"));
    assert_eq!(
        fs::read_to_string(&prompt_path).unwrap(),
        "unmanaged local file\n"
    );

    fs::remove_file(&prompt_path).unwrap();
    adock(&fixture.root)
        .args([
            "distribute",
            "prompt",
            "review",
            "--target",
            "trae",
            "--ref",
            "HEAD",
            "--var",
            "change=PATCH",
            "--yes",
        ])
        .assert()
        .success();
    fs::write(&prompt_path, "managed but locally modified\n").unwrap();

    adock(&fixture.root)
        .args([
            "distribute",
            "prompt",
            "review",
            "--target",
            "trae",
            "--ref",
            "HEAD",
            "--var",
            "change=PATCH",
            "--yes",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("locally_modified"))
        .stderr(predicate::str::contains("--overwrite-managed"));
    assert_eq!(
        fs::read_to_string(&prompt_path).unwrap(),
        "managed but locally modified\n"
    );

    adock(&fixture.root)
        .args([
            "distribute",
            "prompt",
            "review",
            "--target",
            "trae",
            "--ref",
            "HEAD",
            "--var",
            "change=PATCH",
            "--overwrite-managed",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Plan status: ready"))
        .stdout(predicate::str::contains("updated |"));
    assert_eq!(
        fs::read_to_string(&prompt_path).unwrap(),
        "managed but locally modified\n"
    );

    adock(&fixture.root)
        .args([
            "distribute",
            "prompt",
            "review",
            "--target",
            "trae",
            "--ref",
            "HEAD",
            "--var",
            "change=PATCH",
            "--overwrite-managed",
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Execution status: completed"));
    assert!(
        fs::read_to_string(prompt_path)
            .unwrap()
            .contains("Review PATCH in a concise style.")
    );
}

#[test]
fn binary_resources_and_chinese_paths_survive_git_distribution_without_script_execution() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("资产 仓库");
    fs::create_dir(&root).unwrap();
    initialize_repository(&root);
    fs::create_dir_all(root.join("skills/onboard/scripts")).unwrap();
    fs::create_dir_all(root.join("skills/onboard/参考资料")).unwrap();
    fs::write(
        root.join("skills/onboard/SKILL.md"),
        "---\napiVersion: adock/v1alpha1\nkind: skill\nid: onboard\ntitle: 中文入门\ndescription: 验证二进制资源和中文路径\nowners: [bob]\nspec:\n  triggers: [新仓库]\n  dependencies: []\n  entrypoint: scripts/run.cmd\n---\n只复制资源，不执行脚本。\n",
    )
    .unwrap();
    fs::write(
        root.join("skills/onboard/scripts/run.cmd"),
        "@echo off\r\necho executed > script-executed.txt\r\n",
    )
    .unwrap();
    let binary = [0, 159, 255, 10, 0, 128, 76, 54];
    fs::write(root.join("skills/onboard/参考资料/图像.bin"), binary).unwrap();
    commit_all(&root, "binary and unicode assets");
    let commit = head(&root);

    adock(&root)
        .args([
            "distribute",
            "skill",
            "onboard",
            "--target",
            "codex",
            "--ref",
            "HEAD",
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Distribution completed from commit {commit}"
        )));

    let target = root.join("target/codex/.agents/skills/onboard");
    assert_eq!(fs::read(target.join("参考资料/图像.bin")).unwrap(), binary);
    assert!(
        fs::read_to_string(target.join("scripts/run.cmd"))
            .unwrap()
            .contains("echo executed > script-executed.txt")
    );
    assert!(!root.join("script-executed.txt").exists());
    assert!(!target.join("script-executed.txt").exists());
    let manifest =
        fs::read_to_string(root.join("target/codex/.agents/.adock/distribution-manifest.json"))
            .unwrap();
    assert!(manifest.contains(&commit));
    assert!(manifest.contains("skills/onboard/参考资料/图像.bin"));
}

#[test]
fn malformed_variable_arguments_are_redacted_from_stderr() {
    let fixture = Fixture::new();
    let secret = "top-secret-value-that-must-not-leak";

    adock(&fixture.root)
        .args(["show", "prompt", "review", "--var", secret])
        .assert()
        .failure()
        .stderr(predicate::str::contains("argument was redacted"))
        .stderr(predicate::str::contains(secret).not());
}
