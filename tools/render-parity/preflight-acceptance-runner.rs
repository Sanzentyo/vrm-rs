#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
---

//! Preflight checks for an external strict render-parity acceptance runner.

use clap::Parser;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const EXPECTED_THREE_VRM_HEAD: &str = "9d125586f6d7da094b0ac5f204cebf19586f2397";

#[derive(Clone, Debug, Parser)]
#[command(
    name = "preflight-acceptance-runner",
    about = "Check a runner checkout before starting the long render-parity acceptance capture"
)]
struct Options {
    #[arg(long)]
    expected_head: Option<String>,
    #[arg(long, default_value = EXPECTED_THREE_VRM_HEAD)]
    expected_three_vrm_head: String,
    #[arg(long, default_value = ".external-fixtures/three-vrm")]
    three_vrm_root: PathBuf,
    #[arg(long, default_value = ".external-fixtures/official")]
    fixture_dir: PathBuf,
    #[arg(long, default_value = "target/render-parity-acceptance-preflight/ash-shaders")]
    shader_out_dir: PathBuf,
    #[arg(long)]
    skip_shader_compile: bool,
    #[arg(long)]
    allow_dirty: bool,
    #[arg(long, hide = true)]
    self_test: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CheckStatus {
    Pass,
    Fail,
}

#[derive(Clone, Debug)]
struct Check {
    name: &'static str,
    status: CheckStatus,
    detail: String,
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
    let checks = run_checks(&options);
    print_report(&checks);
    if checks.iter().any(|check| check.status == CheckStatus::Fail) {
        return Err("acceptance runner preflight failed".into());
    }
    Ok(())
}

fn run_checks(options: &Options) -> Vec<Check> {
    let mut checks = Vec::new();
    checks.extend(command_checks());
    checks.push(check_git_head(options.expected_head.as_deref()));
    checks.push(check_git_clean(options.allow_dirty));
    checks.push(check_three_vrm_head(
        &options.three_vrm_root,
        &options.expected_three_vrm_head,
    ));
    checks.push(check_file(
        "three_vrm_build",
        &options
            .three_vrm_root
            .join("packages/three-vrm/lib/three-vrm.module.js"),
    ));
    checks.extend(fixture_checks(&options.fixture_dir));
    if options.skip_shader_compile {
        checks.push(pass(
            "ash_shader_compile",
            "skipped by --skip-shader-compile".to_owned(),
        ));
    } else {
        checks.push(check_ash_shader_compile(&options.shader_out_dir));
    }
    checks
}

fn command_checks() -> Vec<Check> {
    [
        ("cargo_nightly", "cargo", &["+nightly", "--version"][..]),
        ("rustc_nightly", "rustc", &["+nightly", "--version"][..]),
        ("just", "just", &["--version"][..]),
        ("node", "node", &["--version"][..]),
        ("npm", npm_program(), &["--version"][..]),
        (
            "glslang_validator",
            "glslangValidator",
            &["--version"][..],
        ),
    ]
    .into_iter()
    .map(|(name, program, args)| check_command(name, program, args))
    .collect()
}

fn check_command(name: &'static str, program: &str, args: &[&str]) -> Check {
    match Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            pass(name, first_line(&stdout).unwrap_or("ok").to_owned())
        }
        Ok(output) => fail(
            name,
            format!(
                "{program} {} exited with {}: {}",
                args.join(" "),
                output.status,
                first_line(&String::from_utf8_lossy(&output.stderr)).unwrap_or("")
            ),
        ),
        Err(error) => fail(name, format!("failed to run {program}: {error}")),
    }
}

fn check_git_head(expected_head: Option<&str>) -> Check {
    match command_stdout("git", &["rev-parse", "HEAD"]) {
        Ok(head) => match expected_head {
            Some(expected) if head.trim() != expected.trim() => fail(
                "repo_head",
                format!("current HEAD {} does not match expected {expected}", head.trim()),
            ),
            _ => pass("repo_head", head.trim().to_owned()),
        },
        Err(error) => fail("repo_head", error),
    }
}

fn check_git_clean(allow_dirty: bool) -> Check {
    if allow_dirty {
        return pass("tracked_worktree_clean", "skipped by --allow-dirty".to_owned());
    }
    match command_stdout("git", &["status", "--porcelain", "--untracked-files=no"]) {
        Ok(status) if status.trim().is_empty() => {
            pass("tracked_worktree_clean", "tracked worktree is clean".to_owned())
        }
        Ok(status) => fail(
            "tracked_worktree_clean",
            format!("tracked changes are present: {}", status.replace('\n', "; ")),
        ),
        Err(error) => fail("tracked_worktree_clean", error),
    }
}

fn check_three_vrm_head(root: &Path, expected_head: &str) -> Check {
    match command_stdout_in(root, "git", &["rev-parse", "HEAD"]) {
        Ok(head) if head.trim() == expected_head => {
            pass("three_vrm_head", head.trim().to_owned())
        }
        Ok(head) => fail(
            "three_vrm_head",
            format!(
                "{} is at {}, expected {expected_head}",
                display_path(root),
                head.trim()
            ),
        ),
        Err(error) => fail("three_vrm_head", error),
    }
}

fn fixture_checks(fixture_dir: &Path) -> Vec<Check> {
    [
        "Seed-san.vrm",
        "VRM1_Constraint_Twist_Sample.vrm",
        "vrm-specification/samples/VRMC_materials_mtoon_UV_Animation_Test/VRMC_materials_mtoon_UV_Animation_Test.vrm",
        "vrm-specification/samples/VRMC_vrm_expressions_isBinary_Overridden/VRMC_vrm_expressions_isBinary_Overridden.vrm",
        "vrm-specification/samples/VRMC_vrm_expressions_isBinary_Overrides/VRMC_vrm_expressions_isBinary_Overrides.vrm",
        "UniVRM/AliciaSolid_vrm-0.51.vrm",
    ]
    .into_iter()
    .map(|relative| {
        check_file(
            "acceptance_fixture",
            &relative.split('/').fold(fixture_dir.to_path_buf(), |path, segment| {
                path.join(segment)
            }),
        )
    })
    .collect()
}

fn check_file(name: &'static str, path: &Path) -> Check {
    if path.is_file() {
        pass(name, display_path(path))
    } else {
        fail(name, format!("missing {}", display_path(path)))
    }
}

fn check_ash_shader_compile(out_dir: &Path) -> Check {
    match Command::new("cargo")
        .args([
            "+nightly",
            "-Zscript",
            "tools/ash/compile-ash-mtoon-base-shaders.rs",
            "--out-dir",
        ])
        .arg(out_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => {
            pass("ash_shader_compile", display_path(out_dir))
        }
        Ok(output) => fail(
            "ash_shader_compile",
            format!(
                "shader compile exited with {}: {}",
                output.status,
                first_line(&String::from_utf8_lossy(&output.stderr)).unwrap_or("")
            ),
        ),
        Err(error) => fail("ash_shader_compile", format!("failed to run cargo: {error}")),
    }
}

fn command_stdout(program: &str, args: &[&str]) -> Result<String, String> {
    command_stdout_with_dir(None, program, args)
}

fn command_stdout_in(dir: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    command_stdout_with_dir(Some(dir), program, args)
}

fn command_stdout_with_dir(
    dir: Option<&Path>,
    program: &str,
    args: &[&str],
) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let output = command.output().map_err(|error| {
        format!(
            "failed to run {}{}: {error}",
            program,
            if let Some(dir) = dir {
                format!(" in {}", display_path(dir))
            } else {
                String::new()
            }
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "{program} {} exited with {}: {}",
            args.join(" "),
            output.status,
            first_line(&String::from_utf8_lossy(&output.stderr)).unwrap_or("")
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn print_report(checks: &[Check]) {
    println!("# Acceptance Runner Preflight");
    println!();
    println!("| Check | Status | Detail |");
    println!("| --- | --- | --- |");
    for check in checks {
        println!(
            "| {} | `{}` | {} |",
            check.name,
            match check.status {
                CheckStatus::Pass => "pass",
                CheckStatus::Fail => "fail",
            },
            check.detail.replace('|', "\\|")
        );
    }
}

fn first_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn pass(name: &'static str, detail: String) -> Check {
    Check {
        name,
        status: CheckStatus::Pass,
        detail,
    }
}

fn fail(name: &'static str, detail: String) -> Check {
    Check {
        name,
        status: CheckStatus::Fail,
        detail,
    }
}

fn npm_program() -> &'static str {
    if cfg!(windows) { "npm.cmd" } else { "npm" }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let checks = vec![
        pass("repo_head", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned()),
        fail("acceptance_fixture", "missing fixture".to_owned()),
    ];
    let failure_count = checks
        .iter()
        .filter(|check| check.status == CheckStatus::Fail)
        .count();
    if failure_count != 1 {
        return Err(format!("expected one failure, got {failure_count}").into());
    }
    if npm_program().is_empty() {
        return Err("npm program should not be empty".into());
    }
    let fixture_count = fixture_checks(Path::new(".external-fixtures/official")).len();
    if fixture_count != 6 {
        return Err(format!("expected six acceptance fixtures, got {fixture_count}").into());
    }
    Ok(())
}
