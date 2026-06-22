#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde_json = "1.0.150"
---

//! Validate that a strict acceptance bundle is fresh for the current checkout.

use clap::Parser;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "validate-current-acceptance-bundle",
    about = "Check that a strict render-parity acceptance bundle is source-locked to the current checkout"
)]
struct Options {
    #[arg(long, default_value = ".external-fixtures/render-parity-acceptance-bundle")]
    bundle: PathBuf,
    #[arg(long)]
    expected_head: Option<String>,
    #[arg(long)]
    allow_dirty: bool,
    #[arg(long, hide = true)]
    self_test: bool,
}

#[derive(Clone, Debug)]
struct BundleFreshness {
    vrm_rs_head: String,
    three_vrm_head: String,
    run_count: u64,
    comparison_count: u64,
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
    let expected_head = match options.expected_head.as_deref() {
        Some(head) => normalized_head(head)?,
        None => {
            if !options.allow_dirty {
                ensure_tracked_worktree_clean()?;
            }
            current_git_head()?
        }
    };
    let freshness = validate_bundle(&options.bundle, &expected_head)?;
    println!(
        "current strict acceptance bundle is fresh: vrm-rs {}, three-vrm {}, runs {}, comparisons {}",
        freshness.vrm_rs_head,
        freshness.three_vrm_head,
        freshness.run_count,
        freshness.comparison_count
    );
    Ok(())
}

fn validate_bundle(
    bundle: &Path,
    expected_head: &str,
) -> Result<BundleFreshness, Box<dyn Error>> {
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
    let vrm_rs_head = required_map_string(source_lock, "vrmRsGitHead")?;
    if vrm_rs_head != expected_head {
        return Err(format!(
            "bundle vrm-rs HEAD {vrm_rs_head} does not match expected HEAD {expected_head}"
        )
        .into());
    }
    if source_lock
        .get("vrmRsGitDirty")
        .and_then(Value::as_bool)
        != Some(false)
    {
        return Err("bundle summary must record vrmRsGitDirty=false".into());
    }
    let three_vrm_head = required_map_string(source_lock, "threeVrmGitHead")?;
    let run_count = required_u64(&summary, "runCount")?;
    let comparison_count = required_array(&summary, "comparisons")?.len() as u64;

    let signoff = fs::read_to_string(bundle.join("acceptance-signoff.md"))?;
    for needle in [
        format!("- vrm-rs HEAD: `{vrm_rs_head}`"),
        format!("- three-vrm HEAD: `{three_vrm_head}`"),
        "- current-source gate: `required`".to_owned(),
        "- numeric gate: pass".to_owned(),
        "- visual review: accepted".to_owned(),
        "- signoff status: complete".to_owned(),
    ] {
        if !signoff.contains(&needle) {
            return Err(format!("acceptance-signoff.md is missing {needle:?}").into());
        }
    }
    if signoff.contains("- reviewer: ``") {
        return Err("acceptance-signoff.md must record a non-empty reviewer".into());
    }

    Ok(BundleFreshness {
        vrm_rs_head: vrm_rs_head.to_owned(),
        three_vrm_head: three_vrm_head.to_owned(),
        run_count,
        comparison_count,
    })
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

fn read_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", display_path(path)).into())
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

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from("target/current-acceptance-bundle-self-test");
    let bundle = root.join("bundle");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&bundle)?;
    let head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    write_bundle(&bundle, head, true, true)?;
    let freshness = validate_bundle(&bundle, head)?;
    if freshness.vrm_rs_head != head || freshness.run_count != 3 || freshness.comparison_count != 1
    {
        return Err("fresh bundle self-test returned unexpected metadata".into());
    }
    if validate_bundle(&bundle, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").is_ok() {
        return Err("bundle with stale source HEAD should be rejected".into());
    }

    write_bundle(&bundle, head, false, true)?;
    if validate_bundle(&bundle, head).is_ok() {
        return Err("non-strict bundle marker should be rejected".into());
    }
    write_bundle(&bundle, head, true, false)?;
    if validate_bundle(&bundle, head).is_ok() {
        return Err("draft signoff should be rejected".into());
    }
    fs::remove_dir_all(&root)?;
    Ok(())
}

fn write_bundle(
    bundle: &Path,
    head: &str,
    strict_marker: bool,
    accepted_signoff: bool,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(bundle)?;
    fs::write(
        bundle.join("bundle-manifest.json"),
        serde_json::json!({
            "bundleFormat": "vrm-rs.render-parity.acceptance-evidence.v1",
            "acceptedSignoffRequired": strict_marker,
            "comparisonCount": 1,
        })
        .to_string(),
    )?;
    fs::write(
        bundle.join("acceptance-repeat-summary.json"),
        serde_json::json!({
            "runCount": 3,
            "comparisons": [
                {
                    "name": "fixture/wgpu",
                    "runs": 3,
                    "minSelectedPsnr": 34.0,
                    "maxAlphaMismatches": 0,
                    "maxAlphaDelta": 0,
                    "maxChannelDelta": 0,
                }
            ],
            "sourceLock": {
                "vrmRsGitHead": head,
                "vrmRsGitDirty": false,
                "threeVrmGitHead": "cccccccccccccccccccccccccccccccccccccccc",
            },
        })
        .to_string(),
    )?;
    let visual_state = if accepted_signoff {
        "accepted"
    } else {
        "pending"
    };
    let status = if accepted_signoff {
        "complete"
    } else {
        "draft"
    };
    fs::write(
        bundle.join("acceptance-signoff.md"),
        format!(
            "# Render Parity Acceptance Signoff\n\n- vrm-rs HEAD: `{head}`\n- current-source gate: `required`\n- three-vrm HEAD: `cccccccccccccccccccccccccccccccccccccccc`\n- numeric gate: pass\n- visual review: {visual_state}\n- signoff status: {status}\n- reviewer: `Codex`\n"
        ),
    )?;
    Ok(())
}
