#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive", "string"] }
serde_json = "1.0.150"
---

//! Generate a threshold calibration and visual-review signoff draft.

use clap::{Parser, ValueEnum};
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "generate-acceptance-signoff",
    about = "Generate render-parity acceptance threshold calibration and visual-review signoff"
)]
struct Options {
    #[arg(long, required_unless_present = "self_test")]
    summary: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    markdown_out: Option<PathBuf>,
    #[arg(long, default_value_t = 3)]
    expected_runs: u64,
    #[arg(long, default_value_t = 18)]
    expected_comparisons: usize,
    #[arg(long, default_value_t = 34.0)]
    min_psnr_floor: f64,
    #[arg(long, default_value_t = 0)]
    max_alpha_mismatches: u64,
    #[arg(long, default_value_t = 0)]
    max_alpha_delta: u64,
    #[arg(long, default_value_t = 60000)]
    expected_browser_ready_timeout_ms: u64,
    #[arg(long, value_enum, default_value_t = VisualReviewState::Pending)]
    visual_review_state: VisualReviewState,
    #[arg(long, default_value = "")]
    reviewer: String,
    #[arg(long, default_value = "")]
    visual_notes: String,
    #[arg(long)]
    require_visual_accepted: bool,
    #[arg(long, hide = true)]
    self_test: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum VisualReviewState {
    Pending,
    Accepted,
    Rejected,
}

impl VisualReviewState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
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

    let summary_path = options
        .summary
        .as_ref()
        .ok_or("--summary is required unless --self-test is supplied")?;
    let summary = read_json(summary_path)?;
    let report = SignoffReport::from_summary(&summary, &options)?;
    let markdown = report.to_markdown(summary_path, &options)?;
    let markdown_out = options
        .markdown_out
        .as_ref()
        .ok_or("--markdown-out is required unless --self-test is supplied")?;
    write_text(markdown_out, &markdown)?;
    if !report.numeric_pass || !report.visual_pass(&options) {
        return Err(format!(
            "acceptance signoff is not complete: numericPass={}, visualState={}",
            report.numeric_pass,
            options.visual_review_state.as_str()
        )
        .into());
    }
    println!("wrote acceptance signoff: {}", display_path(markdown_out));
    Ok(())
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let summary = test_summary(34.25, 0, 0);
    let options = Options {
        summary: None,
        markdown_out: None,
        expected_runs: 3,
        expected_comparisons: 18,
        min_psnr_floor: 34.0,
        max_alpha_mismatches: 0,
        max_alpha_delta: 0,
        expected_browser_ready_timeout_ms: 60000,
        visual_review_state: VisualReviewState::Pending,
        reviewer: String::new(),
        visual_notes: String::new(),
        require_visual_accepted: false,
        self_test: true,
    };
    let report = SignoffReport::from_summary(&summary, &options)?;
    if !report.numeric_pass {
        return Err("passing summary should pass numeric calibration".into());
    }
    let markdown = report.to_markdown(Path::new("summary.json"), &options)?;
    for needle in [
        "numeric gate: pass",
        "visual review: pending",
        "minimum selected PSNR",
        "environment:",
        "gpu adapters:",
    ] {
        if !markdown.contains(needle) {
            return Err(format!("self-test markdown missing {needle:?}").into());
        }
    }

    let low_psnr = test_summary(33.99, 0, 0);
    if SignoffReport::from_summary(&low_psnr, &options)?.numeric_pass {
        return Err("low PSNR summary should fail numeric calibration".into());
    }

    let alpha_fail = test_summary(34.25, 1, 0);
    if SignoffReport::from_summary(&alpha_fail, &options)?.numeric_pass {
        return Err("alpha mismatch summary should fail numeric calibration".into());
    }

    let mut accepted = options.clone();
    accepted.visual_review_state = VisualReviewState::Accepted;
    accepted.reviewer = "reviewer".to_owned();
    accepted.visual_notes = "accepted after visual review".to_owned();
    accepted.require_visual_accepted = true;
    let accepted_report = SignoffReport::from_summary(&summary, &accepted)?;
    if !accepted_report.visual_pass(&accepted) {
        return Err("accepted visual review should satisfy strict visual gate".into());
    }
    accepted.reviewer.clear();
    if accepted_report.visual_pass(&accepted) {
        return Err("accepted visual review should require a reviewer".into());
    }
    accepted.reviewer = "reviewer".to_owned();
    accepted.visual_notes.clear();
    if accepted_report.visual_pass(&accepted) {
        return Err("accepted visual review should require notes".into());
    }

    Ok(())
}

#[derive(Clone, Debug)]
struct SignoffReport {
    run_count: u64,
    comparison_count: usize,
    run_artifacts: Vec<String>,
    environment_summary: Vec<String>,
    source_head: String,
    three_vrm_head: String,
    min_psnr: f64,
    min_psnr_name: String,
    max_alpha_mismatches: u64,
    max_alpha_mismatches_name: String,
    max_alpha_delta: u64,
    max_alpha_delta_name: String,
    max_channel_delta: u64,
    max_channel_delta_name: String,
    numeric_pass: bool,
}

impl SignoffReport {
    fn from_summary(summary: &Value, options: &Options) -> Result<Self, Box<dyn Error>> {
        if required_string(summary, "runMode")? != "acceptance" {
            return Err("summary.runMode must be acceptance for signoff".into());
        }
        if summary.get("referenceClean").and_then(Value::as_bool) != Some(true) {
            return Err("summary.referenceClean must be true for signoff".into());
        }
        let run_count = summary
            .get("runCount")
            .and_then(Value::as_u64)
            .ok_or("summary.runCount must be an integer")?;
        let run_artifacts = required_array(summary, "runs")?
            .iter()
            .map(|run| Ok(required_string(run, "artifacts")?.to_owned()))
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        validate_lane_config(summary, options)?;
        let environment_summary = environment_summary(summary)?;

        let source_lock = required_object(summary, "sourceLock", "summary")?;
        let source_head = required_string(source_lock, "vrmRsGitHead")?.to_owned();
        let three_vrm_head = required_string(source_lock, "threeVrmGitHead")?.to_owned();
        if source_lock.get("vrmRsGitDirty").and_then(Value::as_bool) != Some(false) {
            return Err("sourceLock.vrmRsGitDirty must be false for signoff".into());
        }
        if three_vrm_head != required_string(source_lock, "expectedThreeVrmCommit")? {
            return Err("sourceLock.threeVrmGitHead must match expectedThreeVrmCommit".into());
        }

        let comparisons = required_array(summary, "comparisons")?;
        let mut min_psnr = f64::INFINITY;
        let mut min_psnr_name = String::new();
        let mut max_alpha_mismatches = 0_u64;
        let mut max_alpha_mismatches_name = String::new();
        let mut max_alpha_delta = 0_u64;
        let mut max_alpha_delta_name = String::new();
        let mut max_channel_delta = 0_u64;
        let mut max_channel_delta_name = String::new();
        let mut finite_psnr_count = 0_usize;

        for comparison in comparisons {
            let name = required_string(comparison, "name")?;
            let runs = comparison
                .get("runs")
                .and_then(Value::as_u64)
                .ok_or("comparison.runs must be an integer")?;
            if runs != run_count {
                return Err(format!("{name} has {runs} runs, expected {run_count}").into());
            }
            let psnr = metric_f64(comparison, "minSelectedPsnr")?;
            if psnr.is_finite() {
                finite_psnr_count += 1;
            }
            if psnr < min_psnr {
                min_psnr = psnr;
                min_psnr_name = name.to_owned();
            }
            update_max_u64(
                comparison,
                "maxAlphaMismatches",
                name,
                &mut max_alpha_mismatches,
                &mut max_alpha_mismatches_name,
            )?;
            update_max_u64(
                comparison,
                "maxAlphaDelta",
                name,
                &mut max_alpha_delta,
                &mut max_alpha_delta_name,
            )?;
            update_max_u64(
                comparison,
                "maxChannelDelta",
                name,
                &mut max_channel_delta,
                &mut max_channel_delta_name,
            )?;
        }
        if finite_psnr_count == 0 {
            return Err("at least one comparison must have a finite selected PSNR".into());
        }

        let numeric_pass = run_count == options.expected_runs
            && comparisons.len() == options.expected_comparisons
            && run_artifacts.len() as u64 == run_count
            && min_psnr >= options.min_psnr_floor
            && max_alpha_mismatches <= options.max_alpha_mismatches
            && max_alpha_delta <= options.max_alpha_delta;

        Ok(Self {
            run_count,
            comparison_count: comparisons.len(),
            run_artifacts,
            environment_summary,
            source_head,
            three_vrm_head,
            min_psnr,
            min_psnr_name,
            max_alpha_mismatches,
            max_alpha_mismatches_name,
            max_alpha_delta,
            max_alpha_delta_name,
            max_channel_delta,
            max_channel_delta_name,
            numeric_pass,
        })
    }

    fn visual_pass(&self, options: &Options) -> bool {
        match options.visual_review_state {
            VisualReviewState::Accepted => {
                !options.reviewer.trim().is_empty() && !options.visual_notes.trim().is_empty()
            }
            VisualReviewState::Pending => !options.require_visual_accepted,
            VisualReviewState::Rejected => false,
        }
    }

    fn to_markdown(&self, summary_path: &Path, options: &Options) -> Result<String, Box<dyn Error>> {
        let mut output = String::new();
        output.push_str("# Render Parity Acceptance Signoff\n\n");
        output.push_str(&format!("- Summary: `{}`\n", display_path(summary_path)));
        output.push_str(&format!("- vrm-rs HEAD: `{}`\n", self.source_head));
        output.push_str(&format!("- three-vrm HEAD: `{}`\n", self.three_vrm_head));
        for line in &self.environment_summary {
            output.push_str(&format!("- {line}\n"));
        }
        output.push_str(&format!("- Runs: `{}` / `{}`\n", self.run_count, options.expected_runs));
        output.push_str(&format!(
            "- Comparisons: `{}` / `{}`\n",
            self.comparison_count, options.expected_comparisons
        ));
        output.push_str(&format!(
            "- numeric gate: {}\n",
            if self.numeric_pass { "pass" } else { "fail" }
        ));
        output.push_str(&format!(
            "- visual review: {}\n",
            options.visual_review_state.as_str()
        ));
        output.push_str(&format!(
            "- signoff status: {}\n",
            if self.numeric_pass && self.visual_pass(options) && options.visual_review_state == VisualReviewState::Accepted {
                "complete"
            } else {
                "draft"
            }
        ));
        if !options.reviewer.is_empty() {
            output.push_str(&format!("- reviewer: `{}`\n", options.reviewer));
        }
        if !options.visual_notes.is_empty() {
            output.push_str(&format!("- visual notes: {}\n", options.visual_notes));
        }
        output.push('\n');

        output.push_str("## Artifact Directories\n\n");
        for artifact in &self.run_artifacts {
            output.push_str(&format!("- `{artifact}` (`visual-review.html`)\n"));
        }
        output.push('\n');

        output.push_str("## Threshold Calibration\n\n");
        output.push_str("| Check | Observed | Required | Status |\n");
        output.push_str("| --- | ---: | ---: | --- |\n");
        output.push_str(&format!(
            "| minimum selected PSNR | {:.4} ({}) | >= {:.4} | {} |\n",
            self.min_psnr,
            self.min_psnr_name,
            options.min_psnr_floor,
            pass_fail(self.min_psnr >= options.min_psnr_floor)
        ));
        output.push_str(&format!(
            "| max alpha mismatches | {} ({}) | <= {} | {} |\n",
            self.max_alpha_mismatches,
            empty_name(&self.max_alpha_mismatches_name),
            options.max_alpha_mismatches,
            pass_fail(self.max_alpha_mismatches <= options.max_alpha_mismatches)
        ));
        output.push_str(&format!(
            "| max alpha delta | {} ({}) | <= {} | {} |\n",
            self.max_alpha_delta,
            empty_name(&self.max_alpha_delta_name),
            options.max_alpha_delta,
            pass_fail(self.max_alpha_delta <= options.max_alpha_delta)
        ));
        output.push_str(&format!(
            "| max selected-channel delta | {} ({}) | informational | observe |\n",
            self.max_channel_delta,
            empty_name(&self.max_channel_delta_name),
        ));

        output.push_str("\n## Visual Review Checklist\n\n");
        output.push_str("- Open each `visual-review.html` from the repeated run directories.\n");
        output.push_str("- Confirm pose, silhouette, material placement, broad color, alpha masks, normal-map response, outlines, expressions, and VRM0 orientation.\n");
        output.push_str("- Record any accepted local edge/material residuals with source-level rationale before treating this as final compatibility evidence.\n");
        output.push_str("- Keep generated PNG, diff, `.rgba.json`, and `.imqraw` artifacts external-only unless redistribution is reviewed.\n");
        Ok(output)
    }
}

fn read_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn write_text(path: &Path, text: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)?;
    Ok(())
}

fn validate_lane_config(summary: &Value, options: &Options) -> Result<(), Box<dyn Error>> {
    let lane_config = required_object(summary, "laneConfig", "summary")?;
    for (field, expected) in [
        ("metric", "rgb-visible"),
        ("background", "opaque-black"),
        ("mtoonLightAccumulation", "three-vrm"),
        ("diagnosticMode", "shaded"),
        ("normalMapMode", "generated-tangents"),
    ] {
        let actual = required_string(lane_config, field)?;
        if actual != expected {
            return Err(format!("laneConfig.{field} must be {expected}, got {actual}").into());
        }
    }
    let alpha_mismatch_tolerance = lane_config
        .get("alphaMismatchTolerance")
        .and_then(Value::as_u64)
        .ok_or("laneConfig.alphaMismatchTolerance must be an integer")?;
    if alpha_mismatch_tolerance != options.max_alpha_mismatches {
        return Err(format!(
            "laneConfig.alphaMismatchTolerance ({alpha_mismatch_tolerance}) must match max alpha mismatches ({})",
            options.max_alpha_mismatches
        )
        .into());
    }
    let alpha_channel_tolerance = lane_config
        .get("alphaChannelTolerance")
        .and_then(Value::as_u64)
        .ok_or("laneConfig.alphaChannelTolerance must be an integer")?;
    if alpha_channel_tolerance != options.max_alpha_delta {
        return Err(format!(
            "laneConfig.alphaChannelTolerance ({alpha_channel_tolerance}) must match max alpha delta ({})",
            options.max_alpha_delta
        )
        .into());
    }
    let browser_ready_timeout_ms = lane_config
        .get("browserReadyTimeoutMs")
        .and_then(Value::as_u64)
        .ok_or("laneConfig.browserReadyTimeoutMs must be an integer")?;
    if browser_ready_timeout_ms != options.expected_browser_ready_timeout_ms {
        return Err(format!(
            "laneConfig.browserReadyTimeoutMs ({browser_ready_timeout_ms}) must match expected timeout ({})",
            options.expected_browser_ready_timeout_ms
        )
        .into());
    }
    Ok(())
}

fn environment_summary(summary: &Value) -> Result<Vec<String>, Box<dyn Error>> {
    let environment = required_object(summary, "environmentLock", "summary")?;
    let mut lines = vec![
        format!(
            "environment: `{}` `{}` `{}`",
            required_string(environment, "os")?,
            required_string(environment, "family")?,
            required_string(environment, "arch")?
        ),
    ];
    for (label, field) in [
        ("rustc", "rustcVersion"),
        ("cargo", "cargoVersion"),
        ("node", "nodeVersion"),
        ("npm", "npmVersion"),
        ("just", "justVersion"),
    ] {
        match require_string_or_null(environment, field)? {
            Some(value) => lines.push(format!("{label}: `{value}`")),
            None => lines.push(format!("{label}: `null`")),
        }
    }
    let gpu_adapters = environment
        .get("gpuAdapters")
        .ok_or("environmentLock.gpuAdapters is missing")?;
    if !matches!(gpu_adapters, Value::Null | Value::Array(_)) {
        return Err("environmentLock.gpuAdapters must be null or an array".into());
    }
    lines.push(format!("gpu adapters: `{}`", compact_json(gpu_adapters)?));
    Ok(lines)
}

fn compact_json(value: &Value) -> Result<String, Box<dyn Error>> {
    Ok(serde_json::to_string(value)?)
}

fn update_max_u64(
    value: &Value,
    field: &str,
    name: &str,
    current: &mut u64,
    current_name: &mut String,
) -> Result<(), Box<dyn Error>> {
    let observed = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{field} must be an integer"))?;
    if observed >= *current {
        *current = observed;
        *current_name = name.to_owned();
    }
    Ok(())
}

fn metric_f64(value: &Value, field: &str) -> Result<f64, Box<dyn Error>> {
    match value.get(field) {
        Some(Value::String(value)) if value == "Infinity" => Ok(f64::INFINITY),
        Some(value) => value
            .as_f64()
            .ok_or_else(|| format!("{field} must be a number or Infinity").into()),
        None => Err(format!("{field} must be a number or Infinity").into()),
    }
}

fn required_object<'a>(
    value: &'a Value,
    field: &str,
    source: &str,
) -> Result<&'a Value, Box<dyn Error>> {
    value
        .get(field)
        .filter(|value| value.is_object())
        .ok_or_else(|| format!("{source}.{field} must be an object").into())
}

fn required_array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{field} must be an array").into())
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} must be a string").into())
}

fn require_string_or_null<'a>(
    value: &'a Value,
    field: &str,
) -> Result<Option<&'a str>, Box<dyn Error>> {
    match value.get(field) {
        Some(Value::Null) => Ok(None),
        Some(field_value) => field_value
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("{field} must be a string or null").into()),
        None => Err(format!("{field} must be a string or null").into()),
    }
}

fn pass_fail(pass: bool) -> &'static str {
    if pass { "pass" } else { "fail" }
}

fn empty_name(name: &str) -> &str {
    if name.is_empty() { "none" } else { name }
}

fn test_summary(psnr: f64, alpha_mismatches: u64, alpha_delta: u64) -> Value {
    let comparisons = (0..18)
        .map(|index| {
            serde_json::json!({
                "name": format!("fixture-{index}/renderer"),
                "runs": 3,
                "minSelectedPsnr": psnr,
                "maxChannelDelta": 4,
                "maxAlphaMismatches": alpha_mismatches,
                "maxAlphaDelta": alpha_delta
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "runMode": "acceptance",
        "referenceClean": true,
        "runCount": 3,
        "runs": [
            {"artifacts": ".external-fixtures/render-parity-acceptance-repeat/run-1"},
            {"artifacts": ".external-fixtures/render-parity-acceptance-repeat/run-2"},
            {"artifacts": ".external-fixtures/render-parity-acceptance-repeat/run-3"}
        ],
        "laneConfig": {
            "metric": "rgb-visible",
            "background": "opaque-black",
            "mtoonLightAccumulation": "three-vrm",
            "diagnosticMode": "shaded",
            "normalMapMode": "generated-tangents",
            "alphaMismatchTolerance": 0,
            "alphaChannelTolerance": 0,
            "browserReadyTimeoutMs": 60000
        },
        "sourceLock": {
            "vrmRsGitHead": "0123456789abcdef0123456789abcdef01234567",
            "vrmRsGitDirty": false,
            "threeVrmGitHead": "9d125586f6d7da094b0ac5f204cebf19586f2397",
            "expectedThreeVrmCommit": "9d125586f6d7da094b0ac5f204cebf19586f2397"
        },
        "environmentLock": {
            "os": "windows",
            "family": "windows",
            "arch": "x86_64",
            "rustcVersion": "rustc 1.90.0",
            "cargoVersion": "cargo 1.90.0",
            "nodeVersion": "v25.0.0",
            "npmVersion": null,
            "justVersion": "just 1.42.0",
            "gpuAdapters": [{"Name": "Adapter", "DriverVersion": "1.2.3"}]
        },
        "comparisons": comparisons
    })
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
