#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
---

//! Run the strict render-parity acceptance evidence lane for one runner machine.

use clap::{Parser, ValueEnum};
use std::error::Error;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "run-strict-acceptance-runner",
    about = "Run acceptance-repeat, strict signoff, strict bundle export, and local strict bundle smoke validation"
)]
struct Options {
    #[arg(long, default_value = ".external-fixtures/three-vrm")]
    three_vrm_root: PathBuf,
    #[arg(long, value_enum, default_value_t = RenderBackground::OpaqueBlack)]
    background: RenderBackground,
    #[arg(long, value_enum, default_value_t = LightAccumulation::ThreeVrm)]
    light_accumulation: LightAccumulation,
    #[arg(long, default_value = ".external-fixtures/render-parity-acceptance-repeat")]
    out_root: PathBuf,
    #[arg(long, default_value = ".external-fixtures/render-parity-acceptance-bundle")]
    bundle_out: PathBuf,
    #[arg(long, default_value = ".external-fixtures/render-parity-acceptance-environments")]
    smoke_out_root: PathBuf,
    #[arg(long, default_value_t = 60_000)]
    browser_ready_timeout_ms: u32,
    #[arg(long)]
    reviewer: Option<String>,
    #[arg(long)]
    visual_notes: Option<String>,
    #[arg(long)]
    accept_visual_review: bool,
    #[arg(long)]
    include_visual_contact_sheets: bool,
    #[arg(long)]
    allow_dirty: bool,
    #[arg(long)]
    apply: bool,
    #[arg(long, hide = true)]
    self_test: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RenderBackground {
    OpaqueBlack,
    Transparent,
}

impl RenderBackground {
    fn as_cli_value(self) -> &'static str {
        match self {
            Self::OpaqueBlack => "opaque-black",
            Self::Transparent => "transparent",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum LightAccumulation {
    Tuned,
    ThreeVrm,
}

impl LightAccumulation {
    fn as_cli_value(self) -> &'static str {
        match self {
            Self::Tuned => "tuned",
            Self::ThreeVrm => "three-vrm",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlannedCommand {
    program: String,
    args: Vec<String>,
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
    let commands = plan_commands(&options)?;
    if !options.apply {
        println!("dry run: would run {} commands", commands.len());
        for command in &commands {
            println!("{}", display_command(command));
        }
        println!("rerun with --apply after confirming the runner owns the visual review");
        return Ok(());
    }
    if !options.allow_dirty {
        ensure_tracked_worktree_clean()?;
    }
    for command in &commands {
        run_command(command)?;
    }
    println!(
        "strict acceptance bundle written to {}",
        display_path(&options.bundle_out)
    );
    println!(
        "local strict smoke report written under {}",
        display_path(&options.smoke_out_root)
    );
    Ok(())
}

fn plan_commands(options: &Options) -> Result<Vec<PlannedCommand>, Box<dyn Error>> {
    if !options.accept_visual_review {
        return Err(
            "strict runner requires --accept-visual-review plus reviewer and visual notes".into(),
        );
    }
    let reviewer = required_text(options.reviewer.as_deref(), "--reviewer")?;
    let visual_notes = required_text(options.visual_notes.as_deref(), "--visual-notes")?;
    let summary = options.out_root.join("acceptance-repeat-summary.json");
    let signoff = options.out_root.join("acceptance-signoff.md");
    let smoke_json = options.smoke_out_root.join("local-strict-smoke.json");
    let smoke_markdown = options.smoke_out_root.join("local-strict-smoke.md");
    Ok(vec![
        PlannedCommand {
            program: "just".to_owned(),
            args: vec![
                "render-parity-acceptance-repeat".to_owned(),
                path_arg(&options.three_vrm_root),
                options.background.as_cli_value().to_owned(),
                options.light_accumulation.as_cli_value().to_owned(),
                path_arg(&options.out_root),
                options.browser_ready_timeout_ms.to_string(),
            ],
        },
        PlannedCommand {
            program: "just".to_owned(),
            args: vec![
                "render-parity-acceptance-signoff-strict".to_owned(),
                reviewer.to_owned(),
                visual_notes.to_owned(),
                path_arg(&summary),
                path_arg(&signoff),
            ],
        },
        PlannedCommand {
            program: "just".to_owned(),
            args: vec![
                "render-parity-acceptance-bundle-strict".to_owned(),
                path_arg(&options.out_root),
                path_arg(&options.bundle_out),
                options.include_visual_contact_sheets.to_string(),
            ],
        },
        PlannedCommand {
            program: "cargo".to_owned(),
            args: vec![
                "+nightly".to_owned(),
                "-Zscript".to_owned(),
                "tools/render-parity/validate-acceptance-environments.rs".to_owned(),
                "--bundle".to_owned(),
                path_arg(&options.bundle_out),
                "--min-environments".to_owned(),
                "1".to_owned(),
                "--require-accepted-signoff".to_owned(),
                "--json-out".to_owned(),
                path_arg(&smoke_json),
                "--markdown-out".to_owned(),
                path_arg(&smoke_markdown),
            ],
        },
    ])
}

fn required_text<'a>(value: Option<&'a str>, flag: &str) -> Result<&'a str, Box<dyn Error>> {
    let Some(text) = value.map(str::trim).filter(|text| !text.is_empty()) else {
        return Err(format!("{flag} is required for strict runner signoff").into());
    };
    Ok(text)
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

fn run_command(command: &PlannedCommand) -> Result<(), Box<dyn Error>> {
    println!("running: {}", display_command(command));
    let status = Command::new(&command.program).args(&command.args).status()?;
    if !status.success() {
        return Err(format!(
            "command failed with {}: {}",
            display_status(status),
            display_command(command)
        )
        .into());
    }
    Ok(())
}

fn display_status(status: ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "terminated by signal".to_owned())
}

fn path_arg(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

fn display_path(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

fn display_command(command: &PlannedCommand) -> String {
    std::iter::once(command.program.as_str())
        .chain(command.args.iter().map(String::as_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
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

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let options = Options {
        three_vrm_root: PathBuf::from(".external-fixtures/three-vrm"),
        background: RenderBackground::OpaqueBlack,
        light_accumulation: LightAccumulation::ThreeVrm,
        out_root: PathBuf::from("target/strict-runner/repeat"),
        bundle_out: PathBuf::from("target/strict-runner/bundle"),
        smoke_out_root: PathBuf::from("target/strict-runner/smoke"),
        browser_ready_timeout_ms: 60_000,
        reviewer: Some("Codex".to_owned()),
        visual_notes: Some("accepted after visual review".to_owned()),
        accept_visual_review: true,
        include_visual_contact_sheets: true,
        allow_dirty: false,
        apply: false,
        self_test: true,
    };
    let commands = plan_commands(&options)?;
    if commands.len() != 4 {
        return Err(format!("expected 4 planned commands, got {}", commands.len()).into());
    }
    assert_command(
        &commands[0],
        "just",
        &[
            "render-parity-acceptance-repeat",
            ".external-fixtures/three-vrm",
            "opaque-black",
            "three-vrm",
            "target/strict-runner/repeat",
            "60000",
        ],
    )?;
    assert_command(
        &commands[1],
        "just",
        &[
            "render-parity-acceptance-signoff-strict",
            "Codex",
            "accepted after visual review",
            &path_arg(
                &PathBuf::from("target/strict-runner/repeat")
                    .join("acceptance-repeat-summary.json"),
            ),
            &path_arg(
                &PathBuf::from("target/strict-runner/repeat").join("acceptance-signoff.md"),
            ),
        ],
    )?;
    assert_command(
        &commands[2],
        "just",
        &[
            "render-parity-acceptance-bundle-strict",
            "target/strict-runner/repeat",
            "target/strict-runner/bundle",
            "true",
        ],
    )?;
    assert_command(
        &commands[3],
        "cargo",
        &[
            "+nightly",
            "-Zscript",
            "tools/render-parity/validate-acceptance-environments.rs",
            "--bundle",
            "target/strict-runner/bundle",
            "--min-environments",
            "1",
            "--require-accepted-signoff",
            "--json-out",
            &path_arg(
                &PathBuf::from("target/strict-runner/smoke").join("local-strict-smoke.json"),
            ),
            "--markdown-out",
            &path_arg(
                &PathBuf::from("target/strict-runner/smoke").join("local-strict-smoke.md"),
            ),
        ],
    )?;

    let mut missing_acceptance = options.clone();
    missing_acceptance.accept_visual_review = false;
    if plan_commands(&missing_acceptance).is_ok() {
        return Err("strict runner should require explicit visual-review acceptance".into());
    }
    let mut missing_reviewer = options;
    missing_reviewer.reviewer = Some(" ".to_owned());
    if plan_commands(&missing_reviewer).is_ok() {
        return Err("strict runner should require a non-empty reviewer".into());
    }
    Ok(())
}

fn assert_command(
    command: &PlannedCommand,
    program: &str,
    args: &[&str],
) -> Result<(), Box<dyn Error>> {
    if command.program != program {
        return Err(format!(
            "expected program {program:?}, got {:?}",
            command.program
        )
        .into());
    }
    let expected = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    if command.args != expected {
        return Err(format!("expected args {expected:?}, got {:?}", command.args).into());
    }
    Ok(())
}
