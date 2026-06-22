#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde_json = "1.0.150"
---

//! Generate a small source-locked handoff kit for an external acceptance runner.

use clap::Parser;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_HANDOFF_ROOT: &str = ".external-fixtures/render-parity-acceptance-handoff";
const DEFAULT_OUT_ROOT: &str = ".external-fixtures/render-parity-acceptance-runner-kit";

#[derive(Clone, Debug, Parser)]
#[command(
    name = "generate-acceptance-runner-kit",
    about = "Copy the current render-parity handoff into a small external-runner kit"
)]
struct Options {
    #[arg(long, default_value = DEFAULT_HANDOFF_ROOT)]
    handoff_root: PathBuf,
    #[arg(long, default_value = DEFAULT_OUT_ROOT)]
    out_root: PathBuf,
    #[arg(long)]
    apply: bool,
    #[arg(long, hide = true)]
    self_test: bool,
}

#[derive(Clone, Debug)]
struct HandoffMeta {
    repo_url: String,
    vrm_rs_head: String,
    three_vrm_head: String,
    preflight_command: String,
    capture_command: String,
    finalize_command: String,
    intake_command: String,
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
    let meta = read_handoff_meta(&options.handoff_root.join("handoff.json"))?;
    let files = planned_files(&options.handoff_root, &options.out_root, &meta)?;
    if !options.apply {
        println!("dry run: would write {} files under {}", files.len(), display_path(&options.out_root));
        for file in &files {
            println!("  {}", display_path(&file.path));
        }
        println!("rerun with --apply to write the runner kit");
        return Ok(());
    }
    fs::create_dir_all(&options.out_root)?;
    for file in files {
        if let Some(parent) = file.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file.path, file.contents)?;
    }
    println!("wrote acceptance runner kit: {}", display_path(&options.out_root));
    Ok(())
}

#[derive(Clone, Debug)]
struct PlannedFile {
    path: PathBuf,
    contents: Vec<u8>,
}

fn planned_files(
    handoff_root: &Path,
    out_root: &Path,
    meta: &HandoffMeta,
) -> Result<Vec<PlannedFile>, Box<dyn Error>> {
    let handoff_markdown = fs::read(handoff_root.join("handoff.md"))?;
    let handoff_json = fs::read(handoff_root.join("handoff.json"))?;
    Ok(vec![
        PlannedFile {
            path: out_root.join("README.md"),
            contents: kit_readme(meta).into_bytes(),
        },
        PlannedFile {
            path: out_root.join("handoff.md"),
            contents: handoff_markdown,
        },
        PlannedFile {
            path: out_root.join("handoff.json"),
            contents: handoff_json,
        },
        PlannedFile {
            path: out_root.join("RETURN_BUNDLE_LAYOUT.md"),
            contents: return_layout(meta).into_bytes(),
        },
        PlannedFile {
            path: out_root.join("intake-command.txt"),
            contents: format!("{}\n", meta.intake_command).into_bytes(),
        },
    ])
}

fn read_handoff_meta(path: &Path) -> Result<HandoffMeta, Box<dyn Error>> {
    let value: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    if value.get("format").and_then(Value::as_str)
        != Some("vrm-rs.render-parity.acceptance-handoff.v1")
    {
        return Err(format!("{} is not a render-parity handoff", display_path(path)).into());
    }
    Ok(HandoffMeta {
        repo_url: required_string(&value, "repoUrl")?.to_owned(),
        vrm_rs_head: required_string(&value, "vrmRsGitHead")?.to_owned(),
        three_vrm_head: required_string(&value, "threeVrmGitHead")?.to_owned(),
        preflight_command: required_string(&value, "preflightCommand")?.to_owned(),
        capture_command: required_string(&value, "captureCommand")?.to_owned(),
        finalize_command: required_string(&value, "finalizeCommand")?.to_owned(),
        intake_command: required_string(&value, "returnedBundleIntakeCommand")?.to_owned(),
    })
}

fn kit_readme(meta: &HandoffMeta) -> String {
    format!(
        "# vrm-rs Render Parity Runner Kit\n\n\
         This kit is generated from a source-locked handoff. It contains no fixture binaries or rendered images.\n\n\
         - repo: `{repo}`\n\
         - vrm-rs HEAD: `{head}`\n\
         - expected three-vrm HEAD: `{three}`\n\n\
         ## Runner Flow\n\n\
         1. Follow `handoff.md` to prepare the checkout and external fixtures.\n\
         2. Run preflight before the long capture:\n\n\
         ```powershell\n{preflight}\n```\n\n\
         3. Capture evidence:\n\n\
         ```powershell\n{capture}\n```\n\n\
         4. Inspect the generated visual-review pages, contact sheets, and diff heatmaps.\n\
         5. Finalize only after that review:\n\n\
         ```powershell\n{finalize}\n```\n\n\
         6. Return `.external-fixtures/render-parity-acceptance-bundle/` to the main machine.\n",
        repo = meta.repo_url,
        head = meta.vrm_rs_head,
        three = meta.three_vrm_head,
        preflight = meta.preflight_command,
        capture = meta.capture_command,
        finalize = meta.finalize_command,
    )
}

fn return_layout(meta: &HandoffMeta) -> String {
    format!(
        "# Returned Bundle Layout\n\n\
         Put each returned strict bundle under a distinct sibling directory:\n\n\
         ```text\n\
         .external-fixtures/render-parity-acceptance-returned/\n\
           local/acceptance-bundle/\n\
           external-runner-name/acceptance-bundle/\n\
         ```\n\n\
         Then run final strict intake:\n\n\
         ```powershell\n{intake}\n```\n\n\
         The final intake must see at least two distinct GPU/driver environment signatures.\n",
        intake = meta.intake_command,
    )
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("missing string field {field:?}").into())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from("target/acceptance-runner-kit-self-test");
    let handoff_root = root.join("handoff");
    let out_root = root.join("kit");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&handoff_root)?;
    fs::write(
        handoff_root.join("handoff.md"),
        "# Render Parity Acceptance Handoff\n",
    )?;
    fs::write(
        handoff_root.join("handoff.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "format": "vrm-rs.render-parity.acceptance-handoff.v1",
            "repoUrl": "https://github.com/Sanzentyo/vrm-rs.git",
            "vrmRsGitHead": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "threeVrmGitHead": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "preflightCommand": "just render-parity-acceptance-runner-preflight aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "captureCommand": "just render-parity-acceptance-runner-capture",
            "finalizeCommand": "just render-parity-acceptance-runner-finalize-strict Codex note",
            "returnedBundleIntakeCommand": "just render-parity-acceptance-bundle-root-strict .external-fixtures/render-parity-acceptance-returned"
        }))?,
    )?;
    let meta = read_handoff_meta(&handoff_root.join("handoff.json"))?;
    let files = planned_files(&handoff_root, &out_root, &meta)?;
    if files.len() != 5 {
        return Err(format!("expected 5 planned files, got {}", files.len()).into());
    }
    let readme = String::from_utf8(
        files
            .iter()
            .find(|file| file.path.ends_with("README.md"))
            .ok_or("README.md not planned")?
            .contents
            .clone(),
    )?;
    for needle in [
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "render-parity-acceptance-runner-preflight",
        "render-parity-acceptance-runner-finalize-strict",
    ] {
        if !readme.contains(needle) {
            return Err(format!("README missing {needle:?}").into());
        }
    }
    Ok(())
}
