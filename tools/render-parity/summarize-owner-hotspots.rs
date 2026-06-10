#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Summarize three-vrm `owner-id` hotspot projection reports.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "summarize-owner-hotspots",
    about = "Summarize three-vrm owner-id hotspot projection reports"
)]
struct Options {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerHotspotReport {
    input: String,
    source: Option<String>,
    width: Option<u64>,
    height: Option<u64>,
    sample_center: Option<Vec<f64>>,
    projected_triangle_count: Option<u64>,
    summary: Value,
    rendered_owner_materials: BTreeMap<String, u64>,
    rendered_to_frontmost_materials: BTreeMap<String, u64>,
    rendered_to_best_subpixel_materials: BTreeMap<String, u64>,
    rendered_to_best_neighbor_materials: BTreeMap<String, u64>,
    rendered_to_frontmost_triangles: BTreeMap<String, u64>,
    rendered_to_best_subpixel_triangles: BTreeMap<String, u64>,
    rendered_to_best_neighbor_triangles: BTreeMap<String, u64>,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let input_text = fs::read_to_string(&options.input)?;
    let capture = serde_json::from_str::<Value>(&input_text)?;
    let hotspots = capture
        .pointer("/reference/renderer/diagnosticHotspots")
        .ok_or("missing reference.renderer.diagnosticHotspots")?;
    let report = summarize_report(&options.input, hotspots)?;
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

fn summarize_report(
    input: &Path,
    hotspots: &Value,
) -> Result<OwnerHotspotReport, Box<dyn std::error::Error>> {
    let top = hotspots
        .get("top")
        .and_then(Value::as_array)
        .ok_or("diagnosticHotspots.top must be an array")?;
    let summary = hotspots.get("summary").cloned().unwrap_or(Value::Null);

    let mut rendered_owner_materials = BTreeMap::new();
    let mut rendered_to_frontmost_materials = BTreeMap::new();
    let mut rendered_to_best_subpixel_materials = BTreeMap::new();
    let mut rendered_to_best_neighbor_materials = BTreeMap::new();
    let mut rendered_to_frontmost_triangles = BTreeMap::new();
    let mut rendered_to_best_subpixel_triangles = BTreeMap::new();
    let mut rendered_to_best_neighbor_triangles = BTreeMap::new();

    for hotspot in top {
        let rendered = owner_material(hotspot, "/renderedOwner/owner");
        bump(&mut rendered_owner_materials, rendered.clone());

        let frontmost = candidate_material(hotspot, "frontmost");
        bump_pair(
            &mut rendered_to_frontmost_materials,
            rendered.as_deref(),
            frontmost.as_deref(),
        );
        bump_pair(
            &mut rendered_to_frontmost_triangles,
            rendered_triangle_key(hotspot).as_deref(),
            candidate_triangle_key(hotspot, "frontmost").as_deref(),
        );

        let best_subpixel = hotspot.pointer("/renderedOwnerRecovery/bestSubpixel");
        let best_subpixel_material =
            best_subpixel.and_then(|value| candidate_material(value, "candidate"));
        let best_subpixel_triangle =
            best_subpixel.and_then(|value| candidate_triangle_key(value, "candidate"));
        bump_pair(
            &mut rendered_to_best_subpixel_materials,
            rendered.as_deref(),
            best_subpixel_material.as_deref(),
        );
        bump_pair(
            &mut rendered_to_best_subpixel_triangles,
            rendered_triangle_key(hotspot).as_deref(),
            best_subpixel_triangle.as_deref(),
        );

        let best_neighbor = hotspot.pointer("/renderedOwnerRecovery/bestNeighbor");
        let best_neighbor_material =
            best_neighbor.and_then(|value| candidate_material(value, "candidate"));
        let best_neighbor_triangle =
            best_neighbor.and_then(|value| candidate_triangle_key(value, "candidate"));
        bump_pair(
            &mut rendered_to_best_neighbor_materials,
            rendered.as_deref(),
            best_neighbor_material.as_deref(),
        );
        bump_pair(
            &mut rendered_to_best_neighbor_triangles,
            rendered_triangle_key(hotspot).as_deref(),
            best_neighbor_triangle.as_deref(),
        );
    }

    Ok(OwnerHotspotReport {
        input: display_path(input),
        source: hotspots
            .get("source")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        width: hotspots.get("width").and_then(Value::as_u64),
        height: hotspots.get("height").and_then(Value::as_u64),
        sample_center: hotspots
            .get("sampleCenter")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_f64).collect()),
        projected_triangle_count: hotspots
            .get("projectedTriangleCount")
            .and_then(Value::as_u64),
        summary,
        rendered_owner_materials,
        rendered_to_frontmost_materials,
        rendered_to_best_subpixel_materials,
        rendered_to_best_neighbor_materials,
        rendered_to_frontmost_triangles,
        rendered_to_best_subpixel_triangles,
        rendered_to_best_neighbor_triangles,
    })
}

fn owner_material(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(|owner| owner.get("materialName"))
        .and_then(Value::as_str)
        .map(normalize_material)
}

fn candidate_material(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|candidate| candidate.get("materialName"))
        .and_then(Value::as_str)
        .map(normalize_material)
}

fn rendered_triangle_key(value: &Value) -> Option<String> {
    value.pointer("/renderedOwner/owner")
        .and_then(triangle_key)
}

fn candidate_triangle_key(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(triangle_key)
}

fn triangle_key(value: &Value) -> Option<String> {
    Some(format!(
        "{}|slot{}|tri{}",
        value
            .get("materialName")
            .and_then(Value::as_str)
            .map(normalize_material)
            .unwrap_or_else(|| "unknown-material".to_owned()),
        value.get("materialIndex").and_then(Value::as_i64)?,
        value.get("triangle").and_then(Value::as_i64)?
    ))
}

fn normalize_material(name: &str) -> String {
    name.strip_suffix(":vrm-rs-owner-id-diagnostic")
        .unwrap_or(name)
        .to_owned()
}

fn bump(map: &mut BTreeMap<String, u64>, key: Option<String>) {
    let key = key.unwrap_or_else(|| "none".to_owned());
    *map.entry(key).or_default() += 1;
}

fn bump_pair(map: &mut BTreeMap<String, u64>, left: Option<&str>, right: Option<&str>) {
    let key = format!("{} -> {}", left.unwrap_or("none"), right.unwrap_or("none"));
    *map.entry(key).or_default() += 1;
}

fn markdown_report(report: &OwnerHotspotReport) -> String {
    let mut output = String::new();
    output.push_str("# Owner Hotspot Summary\n\n");
    output.push_str(&format!("- Input: `{}`\n", report.input));
    if let Some(count) = report.projected_triangle_count {
        output.push_str(&format!("- Projected triangles: `{count}`\n"));
    }
    output.push_str("\n## Summary\n\n");
    output.push_str("```json\n");
    output.push_str(
        &serde_json::to_string_pretty(&report.summary).unwrap_or_else(|_| "null".to_owned()),
    );
    output.push_str("\n```\n\n");
    write_top_counts(
        &mut output,
        "Rendered Owner Materials",
        &report.rendered_owner_materials,
    );
    write_top_counts(
        &mut output,
        "Rendered To Frontmost Materials",
        &report.rendered_to_frontmost_materials,
    );
    write_top_counts(
        &mut output,
        "Rendered To Best Subpixel Materials",
        &report.rendered_to_best_subpixel_materials,
    );
    write_top_counts(
        &mut output,
        "Rendered To Best Neighbor Materials",
        &report.rendered_to_best_neighbor_materials,
    );
    output
}

fn write_top_counts(output: &mut String, title: &str, counts: &BTreeMap<String, u64>) {
    output.push_str(&format!("## {title}\n\n"));
    let mut ordered = counts.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
    for (key, count) in ordered.into_iter().take(12) {
        output.push_str(&format!("- `{key}`: `{count}`\n"));
    }
    output.push('\n');
}

fn write_file(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
