#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde_json = "1.0.150"
---

//! Generate a portable handoff note for an external render-parity runner.

use clap::Parser;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_OUT_ROOT: &str = ".external-fixtures/render-parity-acceptance-handoff";
const DEFAULT_THREE_VRM_HEAD: &str = "9d125586f6d7da094b0ac5f204cebf19586f2397";

#[derive(Clone, Debug, Parser)]
#[command(
    name = "generate-acceptance-handoff",
    about = "Write Markdown and JSON instructions for a strict render-parity runner"
)]
struct Options {
    #[arg(long, default_value = DEFAULT_OUT_ROOT)]
    out_root: PathBuf,
    #[arg(long)]
    repo_url: Option<String>,
    #[arg(long)]
    expected_head: Option<String>,
    #[arg(long, default_value = DEFAULT_THREE_VRM_HEAD)]
    three_vrm_head: String,
    #[arg(long, default_value = "vrm-rs-render-runner")]
    checkout_dir: String,
    #[arg(long, default_value = "<runner reviewer>")]
    reviewer: String,
    #[arg(
        long,
        default_value = "Reviewed generated visual review pages and accepted only local edge/outline/text-boundary residuals covered by PSNR and zero-alpha acceptance."
    )]
    visual_notes: String,
    #[arg(long)]
    allow_dirty: bool,
    #[arg(long)]
    apply: bool,
    #[arg(long, hide = true)]
    self_test: bool,
}

#[derive(Clone, Debug)]
struct Handoff {
    repo_url: String,
    expected_head: String,
    three_vrm_head: String,
    checkout_dir: String,
    reviewer: String,
    visual_notes: String,
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
    if !options.allow_dirty {
        ensure_tracked_worktree_clean()?;
    }
    let handoff = Handoff {
        repo_url: options
            .repo_url
            .clone()
            .unwrap_or(current_origin_url()?),
        expected_head: match options.expected_head.as_deref() {
            Some(head) => normalized_head(head)?,
            None => current_git_head()?,
        },
        three_vrm_head: normalized_head(&options.three_vrm_head)?,
        checkout_dir: required_text(&options.checkout_dir, "--checkout-dir")?.to_owned(),
        reviewer: required_text(&options.reviewer, "--reviewer")?.to_owned(),
        visual_notes: required_text(&options.visual_notes, "--visual-notes")?.to_owned(),
    };
    write_or_print_handoff(&handoff, &options.out_root, options.apply)
}

fn write_or_print_handoff(
    handoff: &Handoff,
    out_root: &Path,
    apply: bool,
) -> Result<(), Box<dyn Error>> {
    let json = handoff_json(handoff);
    let markdown = handoff_markdown(handoff);
    if !apply {
        println!(
            "dry run: would write {} and {}",
            display_path(&out_root.join("handoff.json")),
            display_path(&out_root.join("handoff.md"))
        );
        println!("rerun with --apply to write the handoff files");
        println!();
        println!("{markdown}");
        return Ok(());
    }
    fs::create_dir_all(out_root)?;
    fs::write(
        out_root.join("handoff.json"),
        serde_json::to_string_pretty(&json)?,
    )?;
    fs::write(out_root.join("handoff.md"), markdown)?;
    println!("wrote acceptance handoff: {}", display_path(out_root));
    Ok(())
}

fn handoff_json(handoff: &Handoff) -> Value {
    serde_json::json!({
        "format": "vrm-rs.render-parity.acceptance-handoff.v1",
        "repoUrl": handoff.repo_url,
        "vrmRsGitHead": handoff.expected_head,
        "threeVrmGitHead": handoff.three_vrm_head,
        "checkoutDir": handoff.checkout_dir,
        "preflightCommand": preflight_command(handoff),
        "captureCommand": capture_command(),
        "finalizeCommand": finalize_command(handoff),
        "strictRunnerCommand": strict_runner_command(handoff),
        "preparationCommands": preparation_commands(handoff),
        "returnedBundleImportCommand": import_command(),
        "returnedBundleIntakeCommand": "just render-parity-acceptance-bundle-root-strict .external-fixtures/render-parity-acceptance-returned",
    })
}

fn handoff_markdown(handoff: &Handoff) -> String {
    let preparation = preparation_commands(handoff)
        .into_iter()
        .map(|command| format!("{command}\n"))
        .collect::<String>();
    format!(
        "# Render Parity Acceptance Handoff\n\n\
         - repo: `{repo}`\n\
         - vrm-rs HEAD: `{head}`\n\
         - expected three-vrm HEAD: `{three}`\n\
         - returned bundle: `{checkout}/.external-fixtures/render-parity-acceptance-bundle/`\n\n\
         ## Runner Requirements\n\n\
         - Rust nightly with `cargo +nightly -Zscript`.\n\
         - `just`, Node.js, npm, Playwright browser support, and a Vulkan-capable environment for the Ash readback path.\n\
         - Ash MToon shaders are compiled from the source-controlled WGSL ABI through the Rust/Naga tool; no legacy GLSL handoff is required.\n\
         - Enough time to run three repeated six-fixture acceptance captures.\n\n\
         ## Prepare The Runner Checkout\n\n\
         ```powershell\n{preparation}```\n\n\
         ## Preflight\n\n\
         Before starting the long capture, verify the runner checkout, tools, fixtures, three-vrm build, and Ash shader compiler:\n\n\
         ```powershell\n{preflight}\n```\n\n\
         ## Capture\n\n\
         Run the reference-clean acceptance capture first:\n\n\
         ```powershell\n{capture}\n```\n\n\
         ## Review And Finalize\n\n\
         Inspect `.external-fixtures/render-parity-acceptance-repeat/run-*/visual-review.html`, especially run 3, plus the generated contact sheets and diff heatmaps. Only after that review, finalize the bundle:\n\n\
         ```powershell\n{finalize}\n```\n\n\
         The finalize command writes a strict portable bundle under `.external-fixtures/render-parity-acceptance-bundle/` and also runs a local one-environment strict smoke. Return that bundle directory without committing generated images or binary fixtures.\n\n\
         If a runner deliberately wants the older one-command path, the equivalent command is:\n\n\
         ```powershell\n{strict_runner}\n```\n\n\
         ## Intake On The Main Machine\n\n\
         Import each returned strict bundle with a distinct label:\n\n\
         ```powershell\n{import}\n```\n\n\
         This validates the returned bundle directory or `.zip`, copies it under `.external-fixtures/render-parity-acceptance-returned/<label>/acceptance-bundle/`, and runs a one-environment strict smoke. After importing the local bundle and at least one bundle from a distinct GPU/driver environment, run final strict intake:\n\n\
         ```powershell\njust render-parity-acceptance-bundle-root-strict .external-fixtures/render-parity-acceptance-returned\n```\n",
        repo = handoff.repo_url,
        head = handoff.expected_head,
        three = handoff.three_vrm_head,
        checkout = handoff.checkout_dir,
        preflight = preflight_command(handoff),
        capture = capture_command(),
        finalize = finalize_command(handoff),
        strict_runner = strict_runner_command(handoff),
        import = import_command(),
    )
}

fn preparation_commands(handoff: &Handoff) -> Vec<String> {
    vec![
        format!(
            "git clone {} {}",
            shell_quote(&handoff.repo_url),
            shell_quote(&handoff.checkout_dir)
        ),
        format!("cd {}", shell_quote(&handoff.checkout_dir)),
        format!("git fetch origin {}", handoff.expected_head),
        format!("git checkout --detach {}", handoff.expected_head),
        "cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures".to_owned(),
    ]
}

fn strict_runner_command(handoff: &Handoff) -> String {
    format!(
        "just render-parity-acceptance-runner-strict {} {}",
        shell_quote(&handoff.reviewer),
        shell_quote(&handoff.visual_notes)
    )
}

fn preflight_command(handoff: &Handoff) -> String {
    format!(
        "just render-parity-acceptance-runner-preflight {}",
        handoff.expected_head
    )
}

fn capture_command() -> String {
    "just render-parity-acceptance-runner-capture".to_owned()
}

fn finalize_command(handoff: &Handoff) -> String {
    format!(
        "just render-parity-acceptance-runner-finalize-strict {} {}",
        shell_quote(&handoff.reviewer),
        shell_quote(&handoff.visual_notes)
    )
}

fn import_command() -> &'static str {
    "just render-parity-acceptance-import-bundle <returned-bundle-path-or-zip> <runner-label>"
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

fn ensure_tracked_worktree_clean() -> Result<(), Box<dyn Error>> {
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
    if !output.stdout.is_empty() {
        return Err(format!(
            "tracked worktree changes are present; commit/stash them or pass --allow-dirty:\n{}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
        .into());
    }
    Ok(())
}

fn normalized_head(text: &str) -> Result<String, Box<dyn Error>> {
    let head = text.trim();
    if head.len() != 40 || !head.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("expected a 40-character git commit hash, got {head:?}").into());
    }
    Ok(head.to_ascii_lowercase())
}

fn required_text<'a>(text: &'a str, name: &str) -> Result<&'a str, Box<dyn Error>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!("{name} must not be empty").into());
    }
    Ok(trimmed)
}

fn shell_quote(text: &str) -> String {
    if text
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | '\\' | ':'))
    {
        return text.to_owned();
    }
    format!("\"{}\"", text.replace('"', "\\\""))
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let handoff = Handoff {
        repo_url: "https://github.com/Sanzentyo/vrm-rs.git".to_owned(),
        expected_head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        three_vrm_head: DEFAULT_THREE_VRM_HEAD.to_owned(),
        checkout_dir: "vrm-rs-runner".to_owned(),
        reviewer: "Codex".to_owned(),
        visual_notes: "Accepted local residuals".to_owned(),
    };
    let json = handoff_json(&handoff);
    if json.get("format").and_then(Value::as_str)
        != Some("vrm-rs.render-parity.acceptance-handoff.v1")
    {
        return Err("handoff JSON format marker is missing".into());
    }
    if json.get("captureCommand").and_then(Value::as_str)
        != Some("just render-parity-acceptance-runner-capture")
    {
        return Err("handoff JSON capture command is missing".into());
    }
    if json.get("preflightCommand").and_then(Value::as_str)
        != Some(
            "just render-parity-acceptance-runner-preflight aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
    {
        return Err("handoff JSON preflight command is missing".into());
    }
    if json.get("finalizeCommand").and_then(Value::as_str)
        != Some("just render-parity-acceptance-runner-finalize-strict Codex \"Accepted local residuals\"")
    {
        return Err("handoff JSON finalize command is missing".into());
    }
    if json.get("returnedBundleImportCommand").and_then(Value::as_str)
        != Some("just render-parity-acceptance-import-bundle <returned-bundle-path-or-zip> <runner-label>")
    {
        return Err("handoff JSON returned bundle import command is missing".into());
    }
    let markdown = handoff_markdown(&handoff);
    for needle in [
        "git checkout --detach aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures",
        "just render-parity-acceptance-runner-preflight aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "just render-parity-acceptance-runner-capture",
        "just render-parity-acceptance-runner-finalize-strict Codex \"Accepted local residuals\"",
        "just render-parity-acceptance-runner-strict Codex \"Accepted local residuals\"",
        "just render-parity-acceptance-import-bundle <returned-bundle-path-or-zip> <runner-label>",
        "just render-parity-acceptance-bundle-root-strict",
    ] {
        if !markdown.contains(needle) {
            return Err(format!("handoff markdown missing {needle:?}").into());
        }
    }
    if normalized_head("not-a-head").is_ok() {
        return Err("invalid git head should be rejected".into());
    }
    Ok(())
}
