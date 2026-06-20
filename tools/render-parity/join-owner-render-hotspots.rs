#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Join browser owner-id hotspot projections with Rust CPU render hotspot reports.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "join-owner-render-hotspots",
    about = "Join three-vrm owner-id hotspot projections with Rust render hotspot diagnostics"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    owner_hotspots: Option<PathBuf>,
    #[arg(long, required_unless_present = "self_test")]
    rust_hotspots: Option<PathBuf>,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
    #[arg(long, default_value_t = 16)]
    top: usize,
}

#[derive(Clone, Debug, Serialize)]
struct JoinReport {
    owner_hotspots: String,
    rust_hotspots: String,
    owner_hotspot_count: u64,
    rust_hotspot_count: u64,
    joined_count: u64,
    missing_rust_count: u64,
    rendered_owner_count: u64,
    rendered_owner_matches_rust_frontmost: u64,
    rendered_owner_matches_rust_expected_best_subpixel: u64,
    rendered_owner_matches_rust_actual_best_subpixel: u64,
    browser_best_subpixel_count: u64,
    browser_best_subpixel_matches_rust_frontmost: u64,
    browser_best_subpixel_matches_rust_expected_best_subpixel: u64,
    browser_best_subpixel_matches_rust_actual_best_subpixel: u64,
    rendered_to_rust_frontmost: BTreeMap<String, u64>,
    rendered_to_rust_expected_best_subpixel: BTreeMap<String, u64>,
    browser_best_to_rust_expected_best_subpixel: BTreeMap<String, u64>,
    top_disagreements: Vec<JoinedHotspotLine>,
}

#[derive(Clone, Debug, Serialize)]
struct JoinedHotspotLine {
    x: u64,
    y: u64,
    rendered_owner: Option<SurfaceSummary>,
    browser_best_subpixel: Option<SurfaceSummary>,
    browser_best_subpixel_sample: Option<[f64; 2]>,
    browser_rendered_depth_rank: Option<u64>,
    rust_frontmost: Option<SurfaceSummary>,
    rust_expected_best_subpixel: Option<SurfaceSummary>,
    rust_expected_best_subpixel_sample: Option<[f64; 2]>,
    rust_actual_best_subpixel: Option<SurfaceSummary>,
    rust_actual_best_subpixel_sample: Option<[f64; 2]>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
struct SurfaceSummary {
    material_name: String,
    triangle: u64,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    if options.self_test {
        self_test()?;
        return Ok(());
    }
    let owner_path = options.owner_hotspots.as_ref().ok_or("missing --owner-hotspots")?;
    let rust_path = options.rust_hotspots.as_ref().ok_or("missing --rust-hotspots")?;
    let owner = serde_json::from_str::<Value>(&fs::read_to_string(owner_path)?)?;
    let rust = serde_json::from_str::<Value>(&fs::read_to_string(rust_path)?)?;
    let report = join_reports(owner_path, rust_path, &owner, &rust, options.top)?;
    let json = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = &options.json_out {
        write_file(path, &json)?;
    } else {
        print!("{json}");
    }
    if let Some(path) = &options.markdown_out {
        write_file(path, &markdown_report(&report))?;
    }
    Ok(())
}

fn join_reports(
    owner_path: &Path,
    rust_path: &Path,
    owner: &Value,
    rust: &Value,
    top: usize,
) -> Result<JoinReport, Box<dyn std::error::Error>> {
    let owner_hotspots = owner
        .pointer("/reference/renderer/diagnosticHotspots/top")
        .and_then(Value::as_array)
        .ok_or("owner diagnosticHotspots.top must be an array")?;
    let rust_hotspots = rust
        .get("hotspots")
        .and_then(Value::as_array)
        .ok_or("rust hotspots must be an array")?;
    let rust_by_pixel = rust_hotspots
        .iter()
        .filter_map(|hotspot| Some((pixel_key(hotspot)?, hotspot)))
        .collect::<HashMap<_, _>>();

    let mut report = JoinReport {
        owner_hotspots: display_path(owner_path),
        rust_hotspots: display_path(rust_path),
        owner_hotspot_count: owner_hotspots.len() as u64,
        rust_hotspot_count: rust_hotspots.len() as u64,
        joined_count: 0,
        missing_rust_count: 0,
        rendered_owner_count: 0,
        rendered_owner_matches_rust_frontmost: 0,
        rendered_owner_matches_rust_expected_best_subpixel: 0,
        rendered_owner_matches_rust_actual_best_subpixel: 0,
        browser_best_subpixel_count: 0,
        browser_best_subpixel_matches_rust_frontmost: 0,
        browser_best_subpixel_matches_rust_expected_best_subpixel: 0,
        browser_best_subpixel_matches_rust_actual_best_subpixel: 0,
        rendered_to_rust_frontmost: BTreeMap::new(),
        rendered_to_rust_expected_best_subpixel: BTreeMap::new(),
        browser_best_to_rust_expected_best_subpixel: BTreeMap::new(),
        top_disagreements: Vec::new(),
    };

    for owner_hotspot in owner_hotspots {
        let Some((x, y)) = pixel_key(owner_hotspot) else {
            continue;
        };
        let Some(rust_hotspot) = rust_by_pixel.get(&(x, y)).copied() else {
            report.missing_rust_count += 1;
            continue;
        };
        report.joined_count += 1;

        let rendered = owner_surface(owner_hotspot);
        let browser_best = surface_at(owner_hotspot, "/renderedOwnerRecovery/bestSubpixel/candidate");
        let rust_frontmost = surface_at(rust_hotspot, "/frontmost_visible");
        let rust_expected =
            surface_at(rust_hotspot, "/best_subpixel_visible_expected/candidate");
        let rust_actual = surface_at(rust_hotspot, "/best_subpixel_visible_actual/candidate");

        if rendered.is_some() {
            report.rendered_owner_count += 1;
        }
        add_match_counts(
            &mut report.rendered_owner_matches_rust_frontmost,
            rendered.as_ref(),
            rust_frontmost.as_ref(),
        );
        add_match_counts(
            &mut report.rendered_owner_matches_rust_expected_best_subpixel,
            rendered.as_ref(),
            rust_expected.as_ref(),
        );
        add_match_counts(
            &mut report.rendered_owner_matches_rust_actual_best_subpixel,
            rendered.as_ref(),
            rust_actual.as_ref(),
        );

        if browser_best.is_some() {
            report.browser_best_subpixel_count += 1;
        }
        add_match_counts(
            &mut report.browser_best_subpixel_matches_rust_frontmost,
            browser_best.as_ref(),
            rust_frontmost.as_ref(),
        );
        add_match_counts(
            &mut report.browser_best_subpixel_matches_rust_expected_best_subpixel,
            browser_best.as_ref(),
            rust_expected.as_ref(),
        );
        add_match_counts(
            &mut report.browser_best_subpixel_matches_rust_actual_best_subpixel,
            browser_best.as_ref(),
            rust_actual.as_ref(),
        );
        bump_pair(&mut report.rendered_to_rust_frontmost, rendered.as_ref(), rust_frontmost.as_ref());
        bump_pair(
            &mut report.rendered_to_rust_expected_best_subpixel,
            rendered.as_ref(),
            rust_expected.as_ref(),
        );
        bump_pair(
            &mut report.browser_best_to_rust_expected_best_subpixel,
            browser_best.as_ref(),
            rust_expected.as_ref(),
        );

        if report.top_disagreements.len() < top && rendered.as_ref() != rust_expected.as_ref() {
            report.top_disagreements.push(JoinedHotspotLine {
                x,
                y,
                rendered_owner: rendered,
                browser_best_subpixel: browser_best,
                browser_best_subpixel_sample: number_pair(
                    owner_hotspot.pointer("/renderedOwnerRecovery/bestSubpixel/sampleCenter"),
                ),
                browser_rendered_depth_rank: owner_hotspot
                    .get("renderedOwnerDepthRank")
                    .and_then(Value::as_u64),
                rust_frontmost,
                rust_expected_best_subpixel: rust_expected,
                rust_expected_best_subpixel_sample: number_pair(
                    rust_hotspot.pointer("/best_subpixel_visible_expected/sample"),
                ),
                rust_actual_best_subpixel: rust_actual,
                rust_actual_best_subpixel_sample: number_pair(
                    rust_hotspot.pointer("/best_subpixel_visible_actual/sample"),
                ),
            });
        }
    }

    Ok(report)
}

fn add_match_counts(count: &mut u64, left: Option<&SurfaceSummary>, right: Option<&SurfaceSummary>) {
    if left.zip(right).is_some_and(|(left, right)| left == right) {
        *count += 1;
    }
}

fn bump_pair(
    counts: &mut BTreeMap<String, u64>,
    left: Option<&SurfaceSummary>,
    right: Option<&SurfaceSummary>,
) {
    let key = format!("{} -> {}", surface_label(left), surface_label(right));
    *counts.entry(key).or_default() += 1;
}

fn owner_surface(hotspot: &Value) -> Option<SurfaceSummary> {
    surface_at(hotspot, "/renderedOwner/owner")
        .or_else(|| surface_at(hotspot, "/renderedOwnerCandidate"))
}

fn surface_at(value: &Value, pointer: &str) -> Option<SurfaceSummary> {
    let value = value.pointer(pointer)?;
    Some(SurfaceSummary {
        material_name: normalize_material(
            value
                .get("materialName")
                .or_else(|| value.get("material_name"))
                .and_then(Value::as_str)?,
        ),
        triangle: value.get("triangle").and_then(Value::as_u64)?,
    })
}

fn pixel_key(value: &Value) -> Option<(u64, u64)> {
    Some((
        value.get("x").and_then(Value::as_u64)?,
        value.get("y").and_then(Value::as_u64)?,
    ))
}

fn normalize_material(name: &str) -> String {
    name.strip_suffix(":vrm-rs-owner-id-diagnostic")
        .unwrap_or(name)
        .to_owned()
}

fn number_pair(value: Option<&Value>) -> Option<[f64; 2]> {
    let values = value?.as_array()?;
    Some([values.first()?.as_f64()?, values.get(1)?.as_f64()?])
}

fn markdown_report(report: &JoinReport) -> String {
    let mut output = String::new();
    output.push_str("# Joined Owner/Render Hotspots\n\n");
    output.push_str(&format!("- Owner hotspots: `{}`\n", report.owner_hotspots));
    output.push_str(&format!("- Rust hotspots: `{}`\n", report.rust_hotspots));
    output.push_str(&format!(
        "- Joined: `{}` / `{}`; missing Rust: `{}`\n",
        report.joined_count, report.owner_hotspot_count, report.missing_rust_count
    ));
    output.push_str(&format!(
        "- Rendered owner matches Rust frontmost / expected-best / actual-best: `{}` / `{}` / `{}`\n",
        report.rendered_owner_matches_rust_frontmost,
        report.rendered_owner_matches_rust_expected_best_subpixel,
        report.rendered_owner_matches_rust_actual_best_subpixel
    ));
    output.push_str(&format!(
        "- Browser best subpixel matches Rust frontmost / expected-best / actual-best: `{}` / `{}` / `{}`\n\n",
        report.browser_best_subpixel_matches_rust_frontmost,
        report.browser_best_subpixel_matches_rust_expected_best_subpixel,
        report.browser_best_subpixel_matches_rust_actual_best_subpixel
    ));
    output.push_str("## Rendered To Rust Expected Best\n\n");
    write_counts(&mut output, &report.rendered_to_rust_expected_best_subpixel);
    output.push_str("## Browser Best To Rust Expected Best\n\n");
    write_counts(
        &mut output,
        &report.browser_best_to_rust_expected_best_subpixel,
    );
    output.push_str("## Top Rendered/Expected Disagreements\n\n");
    if report.top_disagreements.is_empty() {
        output.push_str("_None_\n");
    } else {
        output.push_str("| Pixel | Rendered | Browser best | Rust frontmost | Rust expected-best | Samples |\n");
        output.push_str("|---|---|---|---|---|---|\n");
        for item in &report.top_disagreements {
            output.push_str(&format!(
                "| {},{} | {} | {} | {} | {} | browser={} rust={} |\n",
                item.x,
                item.y,
                surface_label(item.rendered_owner.as_ref()),
                surface_label(item.browser_best_subpixel.as_ref()),
                surface_label(item.rust_frontmost.as_ref()),
                surface_label(item.rust_expected_best_subpixel.as_ref()),
                fmt_pair(item.browser_best_subpixel_sample),
                fmt_pair(item.rust_expected_best_subpixel_sample),
            ));
        }
    }
    output
}

fn write_counts(output: &mut String, counts: &BTreeMap<String, u64>) {
    let mut ordered = counts.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
    for (key, count) in ordered.into_iter().take(12) {
        output.push_str(&format!("- `{key}`: `{count}`\n"));
    }
    output.push('\n');
}

fn surface_label(surface: Option<&SurfaceSummary>) -> String {
    surface.map_or_else(
        || "none".to_owned(),
        |surface| format!("{}:tri{}", surface.material_name, surface.triangle),
    )
}

fn fmt_pair(value: Option<[f64; 2]>) -> String {
    value.map_or_else(
        || "n/a".to_owned(),
        |value| format!("{:.3},{:.3}", value[0], value[1]),
    )
}

fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn self_test() -> Result<(), Box<dyn std::error::Error>> {
    let owner = serde_json::from_str::<Value>(
        r#"{
            "reference": {"renderer": {"diagnosticHotspots": {"top": [{
                "x": 1,
                "y": 2,
                "renderedOwner": {"owner": {"materialName": "body", "triangle": 7}},
                "renderedOwnerDepthRank": 2,
                "renderedOwnerRecovery": {
                    "bestSubpixel": {
                        "sampleCenter": [0.7, 0.5],
                        "candidate": {"materialName": "body:vrm-rs-owner-id-diagnostic", "triangle": 7}
                    }
                }
            }]}}}
        }"#,
    )?;
    let rust = serde_json::from_str::<Value>(
        r#"{
            "hotspots": [{
                "x": 1,
                "y": 2,
                "frontmost_visible": {"materialName": "hair", "triangle": 3},
                "best_subpixel_visible_expected": {
                    "sample": [0.7, 0.5],
                    "candidate": {"material_name": "body", "triangle": 7}
                },
                "best_subpixel_visible_actual": {
                    "sample": [0.3, 0.5],
                    "candidate": {"material_name": "hair", "triangle": 3}
                }
            }]
        }"#,
    )?;
    let report = join_reports(
        Path::new("owner.json"),
        Path::new("rust.json"),
        &owner,
        &rust,
        8,
    )?;
    assert_eq!(report.joined_count, 1);
    assert_eq!(report.rendered_owner_matches_rust_frontmost, 0);
    assert_eq!(report.rendered_owner_matches_rust_expected_best_subpixel, 1);
    assert_eq!(
        report.browser_best_subpixel_matches_rust_expected_best_subpixel,
        1
    );
    assert!(report.top_disagreements.is_empty());
    let markdown = markdown_report(&report);
    assert!(markdown.contains("Rendered owner matches Rust frontmost"));
    assert!(markdown.contains("body:tri7 -> body:tri7"));
    Ok(())
}
