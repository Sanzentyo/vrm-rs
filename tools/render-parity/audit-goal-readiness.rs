#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde_json = "1.0.150"
---

//! Audit the current evidence against the thread-level render parity goal.

use clap::Parser;
use serde_json::{Value, json};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_LOCAL_BUNDLE: &str = ".external-fixtures/render-parity-acceptance-bundle";
const DEFAULT_HANDOFF: &str = ".external-fixtures/render-parity-acceptance-handoff/handoff.json";
const DEFAULT_ENVIRONMENT_SUMMARY: &str =
    ".external-fixtures/render-parity-acceptance-environments/acceptance-environments-summary.json";

#[derive(Clone, Debug, Parser)]
#[command(
    name = "audit-goal-readiness",
    about = "Report whether current vrm-rs render-parity goal evidence is complete"
)]
struct Options {
    #[arg(long, default_value = DEFAULT_LOCAL_BUNDLE)]
    local_bundle: PathBuf,
    #[arg(long, default_value = DEFAULT_HANDOFF)]
    handoff: PathBuf,
    #[arg(long, default_value = DEFAULT_ENVIRONMENT_SUMMARY)]
    environment_summary: PathBuf,
    #[arg(long, default_value_t = 2)]
    min_environments: u64,
    #[arg(long, default_value_t = 18)]
    expected_comparisons: usize,
    #[arg(long)]
    require_public_repo: bool,
    #[arg(long)]
    repo_url: Option<String>,
    #[arg(long)]
    allow_dirty: bool,
    #[arg(long)]
    fail_if_incomplete: bool,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
    #[arg(long, hide = true)]
    self_test: bool,
}

#[derive(Clone, Debug)]
struct Check {
    name: &'static str,
    status: CheckStatus,
    detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckStatus {
    Pass,
    Missing,
    Fail,
}

impl CheckStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Missing => "missing",
            Self::Fail => "fail",
        }
    }
}

#[derive(Clone, Debug)]
struct AuditReport {
    head: String,
    repo_url: String,
    checks: Vec<Check>,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    if options.self_test {
        return run_self_test();
    }
    let head = current_git_head()?;
    let repo_url = match options.repo_url.as_deref() {
        Some(url) => required_text(url, "--repo-url")?.to_owned(),
        None => current_origin_url()?,
    };
    let report = audit_goal_readiness(&options, &head, &repo_url);
    let complete = report.is_complete();

    let markdown = report.to_markdown();
    if let Some(path) = options.json_out.as_ref() {
        write_json(path, &report.to_json())?;
    }
    if let Some(path) = options.markdown_out.as_ref() {
        write_text(path, &markdown)?;
    }
    print!("{markdown}");

    if options.fail_if_incomplete && !complete {
        return Err("goal readiness audit is incomplete".into());
    }
    Ok(())
}

impl AuditReport {
    fn is_complete(&self) -> bool {
        self.checks
            .iter()
            .all(|check| check.status == CheckStatus::Pass)
    }

    fn to_json(&self) -> Value {
        json!({
            "format": "vrm-rs.render-parity.goal-readiness.v1",
            "complete": self.is_complete(),
            "vrmRsGitHead": self.head,
            "repoUrl": self.repo_url,
            "checks": self.checks.iter().map(|check| {
                json!({
                    "name": check.name,
                    "status": check.status.as_str(),
                    "detail": check.detail,
                })
            }).collect::<Vec<_>>(),
        })
    }

    fn to_markdown(&self) -> String {
        let mut markdown = format!(
            "# Render Parity Goal Readiness\n\n\
             - complete: `{}`\n\
             - vrm-rs HEAD: `{}`\n\
             - repo: `{}`\n\n\
             | Check | Status | Detail |\n\
             | --- | --- | --- |\n",
            self.is_complete(),
            self.head,
            self.repo_url
        );
        for check in &self.checks {
            markdown.push_str(&format!(
                "| {} | `{}` | {} |\n",
                check.name,
                check.status.as_str(),
                escape_markdown_cell(&check.detail)
            ));
        }
        if !self.is_complete() {
            markdown.push_str(
                "\nGoal completion is still unproven. Keep the goal active until every check passes.\n",
            );
        }
        markdown
    }
}

fn audit_goal_readiness(options: &Options, head: &str, repo_url: &str) -> AuditReport {
    let mut checks = Vec::new();
    checks.push(check_clean_worktree(options.allow_dirty));
    checks.push(check_public_repo(repo_url, options.require_public_repo));
    checks.push(check_vrma_parity_surface());
    checks.push(check_non_bevy_adapter_surface());
    checks.push(check_wgpu_ash_material_examples());
    checks.push(check_external_fixture_local_ci());
    checks.push(check_local_bundle(&options.local_bundle, head));
    checks.push(check_handoff(&options.handoff, head, repo_url));
    checks.push(check_environment_summary(
        &options.environment_summary,
        head,
        options.min_environments,
        options.expected_comparisons,
    ));
    AuditReport {
        head: head.to_owned(),
        repo_url: repo_url.to_owned(),
        checks,
    }
}

fn check_clean_worktree(allow_dirty: bool) -> Check {
    if allow_dirty {
        return pass("clean_current_head", "dirty worktree allowed by option");
    }
    match tracked_worktree_status() {
        Ok(status) if status.trim().is_empty() => {
            pass("clean_current_head", "tracked worktree is clean")
        }
        Ok(status) => fail(
            "clean_current_head",
            format!("tracked worktree has changes: {}", one_line(&status)),
        ),
        Err(error) => fail("clean_current_head", error.to_string()),
    }
}

fn check_public_repo(repo_url: &str, require_public_repo: bool) -> Check {
    if !require_public_repo {
        return pass(
            "public_github_repository",
            format!("not checked; pass --require-public-repo to verify {repo_url} with gh"),
        );
    }
    match Command::new("gh")
        .args(["repo", "view", repo_url, "--json", "visibility,url"])
        .output()
    {
        Ok(output) if output.status.success() => match serde_json::from_slice::<Value>(&output.stdout)
        {
            Ok(value) => {
                let visibility = value.get("visibility").and_then(Value::as_str);
                let url = value.get("url").and_then(Value::as_str).unwrap_or(repo_url);
                if visibility == Some("PUBLIC") {
                    pass("public_github_repository", format!("{url} is PUBLIC"))
                } else {
                    fail(
                        "public_github_repository",
                        format!("{url} visibility is {:?}", visibility),
                    )
                }
            }
            Err(error) => fail("public_github_repository", format!("gh JSON parse failed: {error}")),
        },
        Ok(output) => fail(
            "public_github_repository",
            format!(
                "gh repo view failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ),
        Err(error) => fail("public_github_repository", format!("failed to run gh: {error}")),
    }
}

fn check_vrma_parity_surface() -> Check {
    let required_paths = [
        Path::new("examples/headless_vrma_animation.rs"),
        Path::new("examples/bevy_vrma_viewer.rs"),
        Path::new("crates/vrm-adapter-wgpu/examples/vrma_viewer.rs"),
        Path::new("docs/vrma-fixture-discovery.md"),
    ];
    if let Some(path) = required_paths.iter().find(|path| !path.exists()) {
        return missing(
            "vrma_parity_surface",
            format!("missing required VRMA parity surface {}", display_path(path)),
        );
    }
    match read_to_string(Path::new("tools/ci/local-ci.rs")) {
        Ok(ci) => {
            let needles = [
                "headless_vrma_animation",
                "bevy_vrma_viewer",
                "vrma_viewer",
                "--external-fixtures",
            ];
            if let Some(needle) = needles.iter().find(|needle| !ci.contains(*needle)) {
                return fail(
                    "vrma_parity_surface",
                    format!("tools/ci/local-ci.rs does not mention {needle:?}"),
                );
            }
            pass(
                "vrma_parity_surface",
                "headless, Bevy, and wgpu VRMA examples exist and are covered by local CI smokes",
            )
        }
        Err(error) => fail("vrma_parity_surface", error.to_string()),
    }
}

fn check_non_bevy_adapter_surface() -> Check {
    let required_paths = [
        Path::new("examples/custom_engine_adapter.rs"),
        Path::new("crates/vrm-adapter-wgpu/src/lib.rs"),
        Path::new("crates/vrm-adapter-ash/src/lib.rs"),
    ];
    if let Some(path) = required_paths.iter().find(|path| !path.exists()) {
        return missing(
            "non_bevy_engine_adapters",
            format!("missing adapter evidence {}", display_path(path)),
        );
    }
    match read_to_string(Path::new("tools/ci/local-ci.rs")) {
        Ok(ci) if ci.contains("custom_engine_adapter") => pass(
            "non_bevy_engine_adapters",
            "custom engine example plus wgpu/ash adapter crates are present and checked by local CI",
        ),
        Ok(_) => fail(
            "non_bevy_engine_adapters",
            "tools/ci/local-ci.rs does not run custom_engine_adapter",
        ),
        Err(error) => fail("non_bevy_engine_adapters", error.to_string()),
    }
}

fn check_wgpu_ash_material_examples() -> Check {
    let required_paths = [
        Path::new("examples/wgpu_mtoon_pipeline_materialization.rs"),
        Path::new("examples/ash_mtoon_pipeline_materialization.rs"),
        Path::new("examples/mtoon_renderer_skeletons.rs"),
        Path::new("crates/vrm-adapter-ash/shaders/mtoon_base.vert.glsl"),
        Path::new("crates/vrm-adapter-ash/shaders/mtoon_base.frag.glsl"),
    ];
    if let Some(path) = required_paths.iter().find(|path| !path.exists()) {
        return missing(
            "wgpu_ash_material_pipeline_examples",
            format!("missing material pipeline evidence {}", display_path(path)),
        );
    }
    match read_to_string(Path::new("tools/ci/local-ci.rs")) {
        Ok(ci) => {
            let needles = [
                "wgpu_mtoon_pipeline_materialization",
                "ash_mtoon_pipeline_materialization",
                "mtoon_renderer_skeletons",
            ];
            if let Some(needle) = needles.iter().find(|needle| !ci.contains(*needle)) {
                return fail(
                    "wgpu_ash_material_pipeline_examples",
                    format!("tools/ci/local-ci.rs does not run {needle:?}"),
                );
            }
            pass(
                "wgpu_ash_material_pipeline_examples",
                "wgpu/ash MToon materialization examples and Ash shader handoff are present and checked",
            )
        }
        Err(error) => fail("wgpu_ash_material_pipeline_examples", error.to_string()),
    }
}

fn check_external_fixture_local_ci() -> Check {
    if github_actions_workflows_present() {
        return fail(
            "optional_external_fixture_local_ci",
            ".github/workflows contains workflow files, but this repository should use local Rust CI",
        );
    }
    let required_paths = [
        Path::new("tools/ci/local-ci.rs"),
        Path::new("Justfile"),
        Path::new("docs/render-parity.md"),
    ];
    if let Some(path) = required_paths.iter().find(|path| !path.exists()) {
        return missing(
            "optional_external_fixture_local_ci",
            format!("missing local CI evidence {}", display_path(path)),
        );
    }
    let ci = match read_to_string(Path::new("tools/ci/local-ci.rs")) {
        Ok(ci) => ci,
        Err(error) => return fail("optional_external_fixture_local_ci", error.to_string()),
    };
    let justfile = match read_to_string(Path::new("Justfile")) {
        Ok(justfile) => justfile,
        Err(error) => return fail("optional_external_fixture_local_ci", error.to_string()),
    };
    if !ci.contains("--external-fixtures") {
        return fail(
            "optional_external_fixture_local_ci",
            "tools/ci/local-ci.rs does not expose --external-fixtures",
        );
    }
    if !justfile.contains("ci-external:") {
        return fail(
            "optional_external_fixture_local_ci",
            "Justfile does not expose ci-external",
        );
    }
    pass(
        "optional_external_fixture_local_ci",
        "local Rust CI exposes external fixtures and no GitHub Actions workflows are present",
    )
}

fn check_local_bundle(bundle: &Path, head: &str) -> Check {
    match validate_local_bundle(bundle, head) {
        Ok(detail) => pass("current_strict_acceptance_bundle", detail),
        Err(error) => missing("current_strict_acceptance_bundle", error.to_string()),
    }
}

fn validate_local_bundle(bundle: &Path, head: &str) -> Result<String, Box<dyn Error>> {
    let manifest = read_json(&bundle.join("bundle-manifest.json"))?;
    if manifest
        .get("acceptedSignoffRequired")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("bundle-manifest.json must record acceptedSignoffRequired=true".into());
    }
    let summary = read_json(&bundle.join("acceptance-repeat-summary.json"))?;
    let source_lock = required_object(&summary, "sourceLock")?;
    let recorded_head = required_map_string(source_lock, "vrmRsGitHead")?;
    if recorded_head != head {
        return Err(format!("bundle HEAD {recorded_head} does not match current HEAD {head}").into());
    }
    if source_lock
        .get("vrmRsGitDirty")
        .and_then(Value::as_bool)
        != Some(false)
    {
        return Err("bundle summary must record vrmRsGitDirty=false".into());
    }
    let run_count = required_u64(&summary, "runCount")?;
    let comparisons = required_array(&summary, "comparisons")?.len();
    let signoff = fs::read_to_string(bundle.join("acceptance-signoff.md"))?;
    for needle in [
        "- current-source gate: `required`",
        "- numeric gate: pass",
        "- visual review: accepted",
        "- signoff status: complete",
    ] {
        if !signoff.contains(needle) {
            return Err(format!("acceptance-signoff.md is missing {needle:?}").into());
        }
    }
    Ok(format!(
        "{} is source-locked to current HEAD with {run_count} runs and {comparisons} comparisons",
        display_path(bundle)
    ))
}

fn check_handoff(handoff: &Path, head: &str, repo_url: &str) -> Check {
    match validate_handoff(handoff, head, repo_url) {
        Ok(detail) => pass("source_locked_runner_handoff", detail),
        Err(error) => missing("source_locked_runner_handoff", error.to_string()),
    }
}

fn validate_handoff(handoff: &Path, head: &str, repo_url: &str) -> Result<String, Box<dyn Error>> {
    let value = read_json(handoff)?;
    if value.get("format").and_then(Value::as_str)
        != Some("vrm-rs.render-parity.acceptance-handoff.v1")
    {
        return Err("handoff format marker is missing or invalid".into());
    }
    let recorded_head = required_string(&value, "vrmRsGitHead")?;
    if recorded_head != head {
        return Err(format!("handoff HEAD {recorded_head} does not match current HEAD {head}").into());
    }
    let recorded_repo = required_string(&value, "repoUrl")?;
    if normalize_repo_url(recorded_repo) != normalize_repo_url(repo_url) {
        return Err(format!("handoff repo {recorded_repo} does not match {repo_url}").into());
    }
    let preflight_command = required_string(&value, "preflightCommand")?;
    if !preflight_command.contains("render-parity-acceptance-runner-preflight") {
        return Err(
            "handoff preflightCommand must use render-parity-acceptance-runner-preflight".into(),
        );
    }
    let command = required_string(&value, "strictRunnerCommand")?;
    if !command.contains("render-parity-acceptance-runner-strict") {
        return Err("handoff strictRunnerCommand must use render-parity-acceptance-runner-strict".into());
    }
    let capture_command = required_string(&value, "captureCommand")?;
    if !capture_command.contains("render-parity-acceptance-runner-capture") {
        return Err(
            "handoff captureCommand must use render-parity-acceptance-runner-capture".into(),
        );
    }
    let finalize_command = required_string(&value, "finalizeCommand")?;
    if !finalize_command.contains("render-parity-acceptance-runner-finalize-strict") {
        return Err(
            "handoff finalizeCommand must use render-parity-acceptance-runner-finalize-strict"
                .into(),
        );
    }
    Ok(format!(
        "{} targets current HEAD and two-phase strict runner intake",
        display_path(handoff)
    ))
}

fn check_environment_summary(
    summary: &Path,
    head: &str,
    min_environments: u64,
    expected_comparisons: usize,
) -> Check {
    match validate_environment_summary(summary, head, min_environments, expected_comparisons) {
        Ok(detail) => pass("multi_environment_strict_acceptance", detail),
        Err(error) => missing("multi_environment_strict_acceptance", error.to_string()),
    }
}

fn validate_environment_summary(
    summary: &Path,
    head: &str,
    min_environments: u64,
    expected_comparisons: usize,
) -> Result<String, Box<dyn Error>> {
    let value = read_json(summary)?;
    let source_lock = required_object(&value, "sourceLock")?;
    let recorded_head = required_map_string(source_lock, "vrmRsGitHead")?;
    if recorded_head != head {
        return Err(format!(
            "environment summary HEAD {recorded_head} does not match current HEAD {head}"
        )
        .into());
    }
    let environment_count = required_u64(&value, "environmentCount")?;
    if environment_count < min_environments {
        return Err(format!(
            "only {environment_count} distinct environment(s); need at least {min_environments}"
        )
        .into());
    }
    let comparisons = required_array(&value, "comparisons")?;
    if comparisons.len() != expected_comparisons {
        return Err(format!(
            "summary has {} comparisons; expected {expected_comparisons}",
            comparisons.len()
        )
        .into());
    }
    Ok(format!(
        "{} records {environment_count} distinct environments and {} comparisons",
        display_path(summary),
        comparisons.len()
    ))
}

fn current_git_head() -> Result<String, Box<dyn Error>> {
    let output = Command::new("git").args(["rev-parse", "HEAD"]).output()?;
    if !output.status.success() {
        return Err(format!(
            "git rev-parse HEAD failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    normalized_head(String::from_utf8(output.stdout)?.trim())
}

fn current_origin_url() -> Result<String, Box<dyn Error>> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git remote get-url origin failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    required_text(String::from_utf8(output.stdout)?.trim(), "origin remote").map(str::to_owned)
}

fn tracked_worktree_status() -> Result<String, Box<dyn Error>> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git status failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    String::from_utf8(output.stdout).map_err(Into::into)
}

fn normalized_head(text: &str) -> Result<String, Box<dyn Error>> {
    let head = text.trim();
    if head.len() != 40 || !head.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("expected a 40-character git commit hash, got {head:?}").into());
    }
    Ok(head.to_ascii_lowercase())
}

fn normalize_repo_url(url: &str) -> String {
    url.trim()
        .trim_end_matches(".git")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn read_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("{}: {error}", display_path(path)))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", display_path(path)).into())
}

fn read_to_string(path: &Path) -> Result<String, Box<dyn Error>> {
    fs::read_to_string(path).map_err(|error| format!("{}: {error}", display_path(path)).into())
}

fn github_actions_workflows_present() -> bool {
    let workflows = Path::new(".github").join("workflows");
    fs::read_dir(workflows)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .any(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("yml")
                            || extension.eq_ignore_ascii_case("yaml")
                    })
        })
}

fn write_json(path: &Path, value: &Value) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

fn write_text(path: &Path, text: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)?;
    Ok(())
}

fn required_object<'a>(
    value: &'a Value,
    field: &str,
) -> Result<&'a serde_json::Map<String, Value>, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("missing object field {field:?}").into())
}

fn required_map_string<'a>(
    value: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string field {field:?}").into())
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string field {field:?}").into())
}

fn required_u64(value: &Value, field: &str) -> Result<u64, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing unsigned integer field {field:?}").into())
}

fn required_array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing array field {field:?}").into())
}

fn required_text<'a>(text: &'a str, name: &str) -> Result<&'a str, Box<dyn Error>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!("{name} must not be empty").into());
    }
    Ok(trimmed)
}

fn pass(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        status: CheckStatus::Pass,
        detail: detail.into(),
    }
}

fn missing(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        status: CheckStatus::Missing,
        detail: detail.into(),
    }
}

fn fail(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        status: CheckStatus::Fail,
        detail: detail.into(),
    }
}

fn escape_markdown_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

fn one_line(text: &str) -> String {
    text.lines().collect::<Vec<_>>().join("; ")
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from("target/goal-readiness-self-test");
    let _ = fs::remove_dir_all(&root);
    let bundle = root.join("bundle");
    let handoff = root.join("handoff.json");
    let env_summary = root.join("environment-summary.json");
    fs::create_dir_all(&bundle)?;

    let head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    write_json(
        &bundle.join("bundle-manifest.json"),
        &json!({"acceptedSignoffRequired": true}),
    )?;
    write_json(
        &bundle.join("acceptance-repeat-summary.json"),
        &json!({
            "runCount": 3,
            "comparisons": [{}, {}],
            "sourceLock": {
                "vrmRsGitHead": head,
                "vrmRsGitDirty": false,
                "threeVrmGitHead": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            }
        }),
    )?;
    fs::write(
        bundle.join("acceptance-signoff.md"),
        "- current-source gate: `required`\n- numeric gate: pass\n- visual review: accepted\n- signoff status: complete\n- reviewer: `Codex`\n",
    )?;
    write_json(
        &handoff,
        &json!({
            "format": "vrm-rs.render-parity.acceptance-handoff.v1",
            "repoUrl": "https://github.com/Sanzentyo/vrm-rs.git",
            "vrmRsGitHead": head,
            "preflightCommand": "just render-parity-acceptance-runner-preflight aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "captureCommand": "just render-parity-acceptance-runner-capture",
            "finalizeCommand": "just render-parity-acceptance-runner-finalize-strict Codex note",
            "strictRunnerCommand": "just render-parity-acceptance-runner-strict Codex note"
        }),
    )?;
    write_json(
        &env_summary,
        &json!({
            "environmentCount": 2,
            "comparisons": [{}, {}],
            "sourceLock": {"vrmRsGitHead": head}
        }),
    )?;
    fs::create_dir_all(root.join("examples"))?;
    fs::create_dir_all(root.join("crates/vrm-adapter-wgpu/examples"))?;
    fs::create_dir_all(root.join("crates/vrm-adapter-wgpu/src"))?;
    fs::create_dir_all(root.join("crates/vrm-adapter-ash/src"))?;
    fs::create_dir_all(root.join("crates/vrm-adapter-ash/shaders"))?;
    fs::create_dir_all(root.join("docs"))?;
    fs::create_dir_all(root.join("tools/ci"))?;
    for path in [
        "examples/headless_vrma_animation.rs",
        "examples/bevy_vrma_viewer.rs",
        "examples/custom_engine_adapter.rs",
        "examples/wgpu_mtoon_pipeline_materialization.rs",
        "examples/ash_mtoon_pipeline_materialization.rs",
        "examples/mtoon_renderer_skeletons.rs",
        "crates/vrm-adapter-wgpu/examples/vrma_viewer.rs",
        "crates/vrm-adapter-wgpu/src/lib.rs",
        "crates/vrm-adapter-ash/src/lib.rs",
        "crates/vrm-adapter-ash/shaders/mtoon_base.vert.glsl",
        "crates/vrm-adapter-ash/shaders/mtoon_base.frag.glsl",
        "docs/vrma-fixture-discovery.md",
        "docs/render-parity.md",
    ] {
        fs::write(root.join(path), "")?;
    }
    fs::write(
        root.join("tools/ci/local-ci.rs"),
        "headless_vrma_animation bevy_vrma_viewer vrma_viewer --external-fixtures custom_engine_adapter wgpu_mtoon_pipeline_materialization ash_mtoon_pipeline_materialization mtoon_renderer_skeletons",
    )?;
    fs::write(root.join("Justfile"), "ci-external:\n")?;

    let original_dir = std::env::current_dir()?;
    std::env::set_current_dir(&root)?;
    let options = Options {
        local_bundle: PathBuf::from("bundle"),
        handoff: PathBuf::from("handoff.json"),
        environment_summary: PathBuf::from("environment-summary.json"),
        min_environments: 2,
        expected_comparisons: 2,
        require_public_repo: false,
        repo_url: Some("https://github.com/Sanzentyo/vrm-rs".to_owned()),
        allow_dirty: true,
        fail_if_incomplete: false,
        json_out: None,
        markdown_out: None,
        self_test: false,
    };
    let report = audit_goal_readiness(&options, head, "https://github.com/Sanzentyo/vrm-rs.git");
    std::env::set_current_dir(original_dir)?;
    if !report.is_complete() {
        return Err(format!("expected complete self-test report: {}", report.to_markdown()).into());
    }
    let markdown = report.to_markdown();
    for needle in [
        "Render Parity Goal Readiness",
        "current_strict_acceptance_bundle",
        "vrma_parity_surface",
        "non_bevy_engine_adapters",
        "wgpu_ash_material_pipeline_examples",
        "optional_external_fixture_local_ci",
        "multi_environment_strict_acceptance",
        "complete: `true`",
    ] {
        if !markdown.contains(needle) {
            return Err(format!("self-test markdown missing {needle:?}").into());
        }
    }
    Ok(())
}
