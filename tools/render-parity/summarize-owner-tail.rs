#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Summarize the unexplained tail of `compare-owner-id-images.rs` reports.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "summarize-owner-tail",
    about = "Summarize the unexplained tail of owner-id comparison reports"
)]
struct Options {
    #[arg(long, hide = true)]
    self_test: bool,
    #[arg(long, required_unless_present = "self_test")]
    input: Option<PathBuf>,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    markdown_out: Option<PathBuf>,
    #[arg(long, default_value_t = 16)]
    top: usize,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerTailReport {
    input: String,
    expected: Option<String>,
    actual: Option<String>,
    width: Option<u64>,
    height: Option<u64>,
    counts: OwnerTailCounts,
    unexplained_projection_gap_summary: Value,
    expected_raster_bounds_summary: Value,
    actual_raster_bounds_summary: Value,
    expected_raster_metadata_alignment_summary: Value,
    actual_raster_metadata_alignment_summary: Value,
    top_actual_metadata_recoveries: Vec<Value>,
    top_unexplained_material_transitions: Vec<Value>,
    top_unexplained_details: Vec<TailDetail>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct OwnerTailCounts {
    expected_nonzero: Option<u64>,
    actual_nonzero: Option<u64>,
    shared_nonzero: Option<u64>,
    exact_owner_matches: Option<u64>,
    mismatched_shared_nonzero: Option<u64>,
    same_projected_triangle_mismatched_shared_nonzero: Option<u64>,
    same_projected_or_adjacent_triangle_mismatched_shared_nonzero: Option<u64>,
    same_projected_or_touching_triangle_mismatched_shared_nonzero: Option<u64>,
    same_projected_or_adjacent_triangle_near_depth_mismatched_shared_nonzero: Option<u64>,
    unexplained_owner_tail_mismatched_shared_nonzero: Option<u64>,
    unexplained_owner_tail_after_touching_mismatched_shared_nonzero: Option<u64>,
    actual_not_visible_by_cull_policy_mismatched_shared_nonzero: Option<u64>,
    actual_metadata_bounds_miss_mismatched_shared_nonzero: Option<u64>,
    actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero: Option<u64>,
    exact_owner_match_ratio: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
struct TailDetail {
    count: u64,
    bounds: Option<Value>,
    sample_pixels: Vec<Value>,
    expected: OwnerLabelSummary,
    actual: OwnerLabelSummary,
    relation: DetailRelation,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerLabelSummary {
    id: Option<u64>,
    material_name: Option<String>,
    pass: Option<String>,
    mesh_name: Option<String>,
    node_name: Option<String>,
    mesh_index: Option<i64>,
    node_index: Option<i64>,
    primitive_index: Option<i64>,
    material_index: Option<i64>,
    material_slot: Option<u64>,
    triangle: Option<u64>,
    source_triangle: Option<u64>,
    indices: Option<Vec<u64>>,
    render_order: Option<i64>,
    render_phase_order: Option<i64>,
    bevy_phase_order_offset: Option<f64>,
    bevy_phase_order_offset_applied: Option<f64>,
    draw_index: Option<u64>,
    front_face: Option<String>,
    cull_mode: Option<String>,
    alpha_mode: Option<String>,
    depth_write: Option<bool>,
    depth_compare: Option<String>,
    blend: Option<bool>,
    owner_color_source: Option<String>,
    screen_bounds: Option<Value>,
    depth: Option<f64>,
    webgl_depth: Option<f64>,
    reference_webgl_depth: Option<f64>,
    screen_signed_area: Option<f64>,
    front_facing: Option<bool>,
    gpu_front_facing: Option<bool>,
    visible_by_cull_policy: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
struct DetailRelation {
    pass: String,
    material: String,
    mesh: String,
    triangle: String,
    render_order: String,
    render_phase_order: String,
    depth_delta: Option<f64>,
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

    let input = options.input.as_ref().ok_or("missing --input")?;
    let value = serde_json::from_str::<Value>(&fs::read_to_string(input)?)?;
    let report = summarize_report(input, &value, options.top)?;
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
    value: &Value,
    top: usize,
) -> Result<OwnerTailReport, Box<dyn std::error::Error>> {
    let details = value
        .get("top_unexplained_expected_to_actual_details")
        .and_then(Value::as_array)
        .ok_or("top_unexplained_expected_to_actual_details must be an array")?;
    Ok(OwnerTailReport {
        input: display_path(input),
        expected: string_field(value, "expected"),
        actual: string_field(value, "actual"),
        width: u64_field(value, "width"),
        height: u64_field(value, "height"),
        counts: OwnerTailCounts {
            expected_nonzero: u64_field(value, "expected_nonzero"),
            actual_nonzero: u64_field(value, "actual_nonzero"),
            shared_nonzero: u64_field(value, "shared_nonzero"),
            exact_owner_matches: u64_field(value, "exact_owner_matches"),
            mismatched_shared_nonzero: u64_field(value, "mismatched_shared_nonzero"),
            same_projected_triangle_mismatched_shared_nonzero: u64_field(
                value,
                "same_projected_triangle_mismatched_shared_nonzero",
            ),
            same_projected_or_adjacent_triangle_mismatched_shared_nonzero: u64_field(
                value,
                "same_projected_or_adjacent_triangle_mismatched_shared_nonzero",
            ),
            same_projected_or_touching_triangle_mismatched_shared_nonzero: u64_field(
                value,
                "same_projected_or_touching_triangle_mismatched_shared_nonzero",
            ),
            same_projected_or_adjacent_triangle_near_depth_mismatched_shared_nonzero: u64_field(
                value,
                "same_projected_or_adjacent_triangle_near_depth_mismatched_shared_nonzero",
            ),
            unexplained_owner_tail_mismatched_shared_nonzero: u64_field(
                value,
                "unexplained_owner_tail_mismatched_shared_nonzero",
            ),
            unexplained_owner_tail_after_touching_mismatched_shared_nonzero: u64_field(
                value,
                "unexplained_owner_tail_after_touching_mismatched_shared_nonzero",
            ),
            actual_not_visible_by_cull_policy_mismatched_shared_nonzero: u64_field(
                value,
                "actual_not_visible_by_cull_policy_mismatched_shared_nonzero",
            ),
            actual_metadata_bounds_miss_mismatched_shared_nonzero: u64_field(
                value,
                "actual_metadata_bounds_miss_mismatched_shared_nonzero",
            ),
            actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero: u64_field(
                value,
                "actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero",
            ),
            exact_owner_match_ratio: f64_field(value, "exact_owner_match_ratio"),
        },
        unexplained_projection_gap_summary: value
            .get("unexplained_projection_gap_summary")
            .cloned()
            .unwrap_or(Value::Null),
        expected_raster_bounds_summary: value
            .get("expected_raster_bounds_summary")
            .cloned()
            .unwrap_or(Value::Null),
        actual_raster_bounds_summary: value
            .get("actual_raster_bounds_summary")
            .cloned()
            .unwrap_or(Value::Null),
        expected_raster_metadata_alignment_summary: value
            .get("expected_raster_metadata_alignment_summary")
            .cloned()
            .unwrap_or(Value::Null),
        actual_raster_metadata_alignment_summary: value
            .get("actual_raster_metadata_alignment_summary")
            .cloned()
            .unwrap_or(Value::Null),
        top_actual_metadata_recoveries: take_values(value, "top_actual_metadata_recoveries", top),
        top_unexplained_material_transitions: take_values(
            value,
            "top_unexplained_material_transitions",
            top,
        ),
        top_unexplained_details: details
            .iter()
            .take(top)
            .map(summarize_detail)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn summarize_detail(value: &Value) -> Result<TailDetail, Box<dyn std::error::Error>> {
    let expected = label_summary(value.get("expected").ok_or("detail missing expected")?);
    let actual = label_summary(value.get("actual").ok_or("detail missing actual")?);
    Ok(TailDetail {
        count: value.get("count").and_then(Value::as_u64).unwrap_or(0),
        bounds: value.get("bounds").cloned(),
        sample_pixels: value
            .get("sample_pixels")
            .and_then(Value::as_array)
            .map(|pixels| pixels.iter().cloned().take(8).collect())
            .unwrap_or_default(),
        relation: detail_relation(&expected, &actual),
        expected,
        actual,
    })
}

fn label_summary(value: &Value) -> OwnerLabelSummary {
    OwnerLabelSummary {
        id: u64_field(value, "id"),
        material_name: string_field(value, "material_name"),
        pass: string_field(value, "pass"),
        mesh_name: string_field(value, "mesh_name"),
        node_name: string_field(value, "node_name"),
        mesh_index: i64_field(value, "mesh_index"),
        node_index: i64_field(value, "node_index"),
        primitive_index: i64_field(value, "primitive_index"),
        material_index: i64_field(value, "material_index"),
        material_slot: u64_field(value, "material_slot"),
        triangle: u64_field(value, "triangle"),
        source_triangle: u64_field(value, "source_triangle"),
        indices: value
            .get("indices")
            .and_then(Value::as_array)
            .map(|indices| indices.iter().filter_map(Value::as_u64).collect()),
        render_order: i64_field(value, "render_order"),
        render_phase_order: i64_field(value, "render_phase_order"),
        bevy_phase_order_offset: f64_field(value, "bevy_phase_order_offset"),
        bevy_phase_order_offset_applied: f64_field(value, "bevy_phase_order_offset_applied"),
        draw_index: u64_field(value, "draw_index"),
        front_face: string_field(value, "front_face"),
        cull_mode: string_field(value, "cull_mode"),
        alpha_mode: string_field(value, "alpha_mode"),
        depth_write: bool_field(value, "depth_write"),
        depth_compare: string_field(value, "depth_compare"),
        blend: bool_field(value, "blend"),
        owner_color_source: string_field(value, "owner_color_source"),
        screen_bounds: value.get("screen_bounds").cloned(),
        depth: f64_field(value, "depth"),
        webgl_depth: f64_field(value, "webgl_depth"),
        reference_webgl_depth: f64_field(value, "reference_webgl_depth"),
        screen_signed_area: f64_field(value, "screen_signed_area"),
        front_facing: bool_field(value, "front_facing"),
        gpu_front_facing: bool_field(value, "gpu_front_facing"),
        visible_by_cull_policy: bool_field(value, "visible_by_cull_policy"),
    }
}

fn detail_relation(expected: &OwnerLabelSummary, actual: &OwnerLabelSummary) -> DetailRelation {
    DetailRelation {
        pass: relation_label(expected.pass.as_deref(), actual.pass.as_deref()),
        material: if expected
            .material_index
            .zip(actual.material_index)
            .is_some_and(|(left, right)| left == right)
        {
            "same-index".to_owned()
        } else {
            relation_label(
                expected.material_name.as_deref(),
                actual.material_name.as_deref(),
            )
        },
        mesh: mesh_relation(expected, actual),
        triangle: triangle_relation(expected, actual),
        render_order: order_relation_i64(expected.render_order, actual.render_order),
        render_phase_order: order_relation_i64(
            expected.render_phase_order,
            actual.render_phase_order,
        ),
        depth_delta: expected
            .reference_webgl_depth
            .or(expected.webgl_depth)
            .or(expected.depth)
            .zip(
                actual
                    .reference_webgl_depth
                    .or(actual.webgl_depth)
                    .or(actual.depth),
            )
            .map(|(expected, actual)| expected - actual),
    }
}

fn mesh_relation(expected: &OwnerLabelSummary, actual: &OwnerLabelSummary) -> String {
    if expected
        .mesh_index
        .zip(actual.mesh_index)
        .is_some_and(|(left, right)| left == right)
    {
        return "same-index".to_owned();
    }
    if same_nonempty(expected.mesh_name.as_deref(), actual.mesh_name.as_deref()) {
        return "same-name".to_owned();
    }
    if normalized_mesh_name(expected.mesh_name.as_deref())
        == normalized_mesh_name(actual.mesh_name.as_deref())
    {
        return "same-normalized-name".to_owned();
    }
    "different".to_owned()
}

fn triangle_relation(expected: &OwnerLabelSummary, actual: &OwnerLabelSummary) -> String {
    if expected
        .triangle
        .zip(actual.triangle)
        .is_some_and(|(left, right)| left == right)
    {
        return "same-triangle".to_owned();
    }
    if let Some(shared) = expected
        .indices
        .as_deref()
        .zip(actual.indices.as_deref())
        .map(|(left, right)| left.iter().filter(|index| right.contains(index)).count())
    {
        return match shared {
            2.. => "shared-edge-indices".to_owned(),
            1 => "shared-vertex-indices".to_owned(),
            _ => "different-triangle".to_owned(),
        };
    }
    "unknown".to_owned()
}

fn normalized_mesh_name(name: Option<&str>) -> Option<String> {
    let raw = name?;
    let name = raw.strip_suffix("_primitive").unwrap_or(raw);
    Some(
        name.rsplit_once('_')
            .and_then(|(prefix, suffix)| suffix.parse::<u32>().ok().map(|_| prefix))
            .unwrap_or(name)
            .to_owned(),
    )
}

fn relation_label(left: Option<&str>, right: Option<&str>) -> String {
    match (left, right) {
        (Some(left), Some(right)) if left == right => "same".to_owned(),
        (Some(_), Some(_)) => "different".to_owned(),
        _ => "unknown".to_owned(),
    }
}

fn order_relation_i64(left: Option<i64>, right: Option<i64>) -> String {
    match left.zip(right) {
        Some((left, right)) if left == right => "same".to_owned(),
        Some((left, right)) if left < right => "expected-before-actual".to_owned(),
        Some(_) => "expected-after-actual".to_owned(),
        None => "unknown".to_owned(),
    }
}

fn same_nonempty(left: Option<&str>, right: Option<&str>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if !left.is_empty() && left == right)
}

fn take_values(value: &Value, key: &str, top: usize) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().take(top).cloned().collect())
        .unwrap_or_default()
}

fn markdown_report(report: &OwnerTailReport) -> String {
    let mut output = String::new();
    output.push_str("# Owner Tail Summary\n\n");
    output.push_str(&format!("- Input: `{}`\n", report.input));
    if let Some(expected) = &report.expected {
        output.push_str(&format!("- Expected: `{expected}`\n"));
    }
    if let Some(actual) = &report.actual {
        output.push_str(&format!("- Actual: `{actual}`\n"));
    }
    if let Some(tail) = report
        .counts
        .unexplained_owner_tail_mismatched_shared_nonzero
    {
        output.push_str(&format!("- Unexplained owner tail: `{tail}`\n"));
    }
    if let Some(tail) = report
        .counts
        .unexplained_owner_tail_after_touching_mismatched_shared_nonzero
    {
        output.push_str(&format!(
            "- Tail after touching-triangle filter: `{tail}`\n"
        ));
    }

    output.push_str("\n## Key Counts\n\n");
    output.push_str("| Metric | Count |\n|---|---:|\n");
    write_count(
        &mut output,
        "mismatched_shared_nonzero",
        report.counts.mismatched_shared_nonzero,
    );
    write_count(
        &mut output,
        "same_projected_triangle_mismatched_shared_nonzero",
        report
            .counts
            .same_projected_triangle_mismatched_shared_nonzero,
    );
    write_count(
        &mut output,
        "same_projected_or_touching_triangle_mismatched_shared_nonzero",
        report
            .counts
            .same_projected_or_touching_triangle_mismatched_shared_nonzero,
    );
    write_count(
        &mut output,
        "unexplained_owner_tail_mismatched_shared_nonzero",
        report
            .counts
            .unexplained_owner_tail_mismatched_shared_nonzero,
    );
    write_count(
        &mut output,
        "unexplained_owner_tail_after_touching_mismatched_shared_nonzero",
        report
            .counts
            .unexplained_owner_tail_after_touching_mismatched_shared_nonzero,
    );
    write_count(
        &mut output,
        "actual_not_visible_by_cull_policy_mismatched_shared_nonzero",
        report
            .counts
            .actual_not_visible_by_cull_policy_mismatched_shared_nonzero,
    );
    write_count(
        &mut output,
        "actual_metadata_bounds_miss_mismatched_shared_nonzero",
        report
            .counts
            .actual_metadata_bounds_miss_mismatched_shared_nonzero,
    );
    write_count(
        &mut output,
        "actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero",
        report
            .counts
            .actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero,
    );

    output.push_str("\n## Projection Gap Shape\n\n");
    output.push_str("| Metric | Count |\n|---|---:|\n");
    write_projection_gap_count(&mut output, report, "with_screen_bounds");
    write_projection_gap_count(&mut output, report, "overlapping_screen_bounds_1px");
    write_projection_gap_count(&mut output, report, "disjoint_screen_bounds_1px");
    write_projection_gap_count(&mut output, report, "pixel_near_either_edge_025px");
    write_projection_gap_count(&mut output, report, "pixel_near_both_edges_025px");
    write_projection_gap_count(&mut output, report, "pixel_near_either_edge_05px");
    write_projection_gap_count(&mut output, report, "pixel_near_both_edges_05px");
    write_projection_gap_count(&mut output, report, "pixel_inside_expected_screen_bounds");
    write_projection_gap_count(&mut output, report, "pixel_inside_actual_screen_bounds");
    write_projection_gap_count(&mut output, report, "pixel_inside_both_screen_bounds");
    write_projection_gap_count(&mut output, report, "pixel_inside_expected_only_screen_bounds");
    write_projection_gap_count(&mut output, report, "pixel_inside_actual_only_screen_bounds");
    write_projection_gap_count(&mut output, report, "pixel_inside_neither_screen_bounds");
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_inside_expected_only_within_actual_bounds_025px",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_inside_expected_only_within_actual_bounds_05px",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_inside_expected_only_within_actual_bounds_1px",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_inside_expected_only_within_actual_bounds_2px",
    );
    write_projection_gap_value(
        &mut output,
        report,
        "mean_expected_only_distance_to_actual_bounds",
    );
    write_projection_gap_value(
        &mut output,
        report,
        "max_expected_only_distance_to_actual_bounds",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_inside_actual_only_within_expected_bounds_025px",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_inside_actual_only_within_expected_bounds_05px",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_inside_actual_only_within_expected_bounds_1px",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_inside_actual_only_within_expected_bounds_2px",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_expected_screen_bounds",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_actual_screen_bounds",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_both_screen_bounds",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_expected_only_screen_bounds",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_actual_only_screen_bounds",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_neither_screen_bounds",
    );
    write_projection_gap_count(&mut output, report, "with_screen_triangles");
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_center_inside_expected_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_center_inside_actual_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_center_inside_both_screen_triangles",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_center_inside_expected_only_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_center_inside_actual_only_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_center_inside_neither_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_expected_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_actual_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_both_screen_triangles",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_expected_only_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_actual_only_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_origin_inside_neither_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_subpixel3_inside_expected_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_subpixel3_inside_actual_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_subpixel3_inside_both_screen_triangles",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_subpixel3_inside_expected_only_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_subpixel3_inside_actual_only_screen_triangle",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "pixel_subpixel3_inside_neither_screen_triangle",
    );
    write_projection_gap_value(
        &mut output,
        report,
        "mean_actual_only_distance_to_expected_bounds",
    );
    write_projection_gap_value(
        &mut output,
        report,
        "max_actual_only_distance_to_expected_bounds",
    );
    write_projection_gap_count(&mut output, report, "pixel_near_expected_min_x_edge_05px");
    write_projection_gap_count(&mut output, report, "pixel_near_expected_max_x_edge_05px");
    write_projection_gap_count(&mut output, report, "pixel_near_expected_min_y_edge_05px");
    write_projection_gap_count(&mut output, report, "pixel_near_expected_max_y_edge_05px");
    write_projection_gap_count(&mut output, report, "pixel_near_actual_min_x_edge_05px");
    write_projection_gap_count(&mut output, report, "pixel_near_actual_max_x_edge_05px");
    write_projection_gap_count(&mut output, report, "pixel_near_actual_min_y_edge_05px");
    write_projection_gap_count(&mut output, report, "pixel_near_actual_max_y_edge_05px");
    write_projection_gap_count(&mut output, report, "either_small_bounds_area_le_1px");
    write_projection_gap_count(&mut output, report, "both_small_bounds_area_le_1px");
    write_projection_gap_count(&mut output, report, "either_small_bounds_area_le_4px");
    write_projection_gap_count(&mut output, report, "both_small_bounds_area_le_4px");
    write_projection_gap_count(
        &mut output,
        report,
        "with_expected_bevy_phase_order_offset_applied",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "expected_bevy_phase_order_offset_applied_nonzero",
    );
    write_projection_gap_value(
        &mut output,
        report,
        "mean_expected_bevy_phase_order_offset_applied",
    );
    write_projection_gap_value(
        &mut output,
        report,
        "max_expected_bevy_phase_order_offset_applied",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "with_actual_bevy_phase_order_offset_applied",
    );
    write_projection_gap_count(
        &mut output,
        report,
        "actual_bevy_phase_order_offset_applied_nonzero",
    );
    write_projection_gap_value(
        &mut output,
        report,
        "mean_actual_bevy_phase_order_offset_applied",
    );
    write_projection_gap_value(
        &mut output,
        report,
        "max_actual_bevy_phase_order_offset_applied",
    );

    output.push_str("\n## Raster Bounds vs Metadata\n\n");
    output.push_str("| Image | Metric | Value |\n|---|---|---:|\n");
    write_raster_bounds_summary(&mut output, "expected", &report.expected_raster_bounds_summary);
    write_raster_bounds_summary(&mut output, "actual", &report.actual_raster_bounds_summary);

    output.push_str("\n## Top Actual Raster Bounds Excess\n\n");
    output.push_str("| Owner | Pixels | Center Excess | Origin Excess |\n|---:|---:|---:|---:|\n");
    for gap in report
        .actual_raster_bounds_summary
        .get("top_center_bounds_excess")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(8)
    {
        output.push_str(&format!(
            "| {} | {} | {:.6} | {:.6} |\n",
            u64_field(gap, "owner").unwrap_or(0),
            u64_field(gap, "pixels").unwrap_or(0),
            gap.pointer("/center_excess/max")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            gap.pointer("/origin_excess/max")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
        ));
    }

    output.push_str("\n## Raster Metadata Alignment\n\n");
    output.push_str("| Image | Metric | Value |\n|---|---|---:|\n");
    write_raster_alignment_summary(
        &mut output,
        "expected",
        &report.expected_raster_metadata_alignment_summary,
    );
    write_raster_alignment_summary(
        &mut output,
        "actual",
        &report.actual_raster_metadata_alignment_summary,
    );

    output.push_str("\n## Top Actual Metadata Recoveries\n\n");
    output.push_str(
        "| Count | Decoded | Recovered | ID Delta | RGB Delta | Class | Source Δ | Draw Δ | Relation |\n|---:|---:|---:|---:|---|---|---:|---:|---|\n",
    );
    for item in &report.top_actual_metadata_recoveries {
        output.push_str(&format!(
            "| {} | {} | {} | {} | ({:+}, {:+}, {:+}) | {} | {} | {} | {}; {}; {}; {} |\n",
            u64_field(item, "count").unwrap_or(0),
            u64_field(item, "decoded_actual").unwrap_or(0),
            u64_field(item, "recovered_actual").unwrap_or(0),
            i64_field(item, "id_delta").unwrap_or(0),
            i64_field(item, "red_delta").unwrap_or(0),
            i64_field(item, "green_delta").unwrap_or(0),
            i64_field(item, "blue_delta").unwrap_or(0),
            text_field(item, "channel_delta_class"),
            optional_i64_cell(item, "source_triangle_delta"),
            optional_i64_cell(item, "draw_index_delta"),
            text_field(item, "mesh_relation"),
            text_field(item, "material_relation"),
            text_field(item, "triangle_relation"),
            text_field(item, "projection_relation"),
        ));
    }

    output.push_str("\n## Top Actual Raster Metadata Reassignments\n\n");
    output.push_str(
        "| Owner | Pixels | Self Distance | Best Owner | Best Distance | Delta | Owner Mesh | Best Mesh |\n|---:|---:|---:|---:|---:|---:|---|---|\n",
    );
    for item in report
        .actual_raster_metadata_alignment_summary
        .get("top_aligned_elsewhere")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(8)
    {
        output.push_str(&format!(
            "| {} | {} | {:.6} | {} | {:.6} | {} | {} | {} |\n",
            u64_field(item, "owner").unwrap_or(0),
            u64_field(item, "pixels").unwrap_or(0),
            item.get("self_center_distance")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            u64_field(item, "best_owner").unwrap_or(0),
            item.get("best_center_distance")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            i64_field(item, "owner_delta").unwrap_or(0),
            markdown_text_field(item, "owner_mesh_name"),
            markdown_text_field(item, "best_mesh_name"),
        ));
    }

    output.push_str("\n## Top Unexplained Material Transitions\n\n");
    output.push_str("| Count | Expected | Actual | Relation |\n|---:|---|---|---|\n");
    for transition in &report.top_unexplained_material_transitions {
        output.push_str(&format!(
            "| {} | {} / {} / {} | {} / {} / {} | {}; {}; {} |\n",
            u64_field(transition, "count").unwrap_or(0),
            text_field(transition, "expected_pass"),
            text_field(transition, "expected_mesh"),
            text_field(transition, "expected_material"),
            text_field(transition, "actual_pass"),
            text_field(transition, "actual_mesh"),
            text_field(transition, "actual_material"),
            text_field(transition, "material_relation"),
            text_field(transition, "triangle_relation"),
            text_field(transition, "projection_relation"),
        ));
    }

    output.push_str("\n## Top Unexplained Details\n\n");
    output.push_str("| Count | Expected | Actual | Relation | Pixels |\n|---:|---|---|---|---|\n");
    for detail in &report.top_unexplained_details {
        output.push_str(&format!(
            "| {} | {} | {} | {}; {}; {}; depth_delta={} | {} |\n",
            detail.count,
            label_cell(&detail.expected),
            label_cell(&detail.actual),
            detail.relation.pass,
            detail.relation.mesh,
            detail.relation.triangle,
            detail
                .relation
                .depth_delta
                .map(|value| format!("{value:.6}"))
                .unwrap_or_else(|| "n/a".to_owned()),
            pixels_cell(&detail.sample_pixels),
        ));
    }
    output
}

fn label_cell(label: &OwnerLabelSummary) -> String {
    let bevy_phase = label
        .bevy_phase_order_offset_applied
        .map(|value| format!(" / bevy_phase={value:.8}"))
        .unwrap_or_default();
    let owner_color = label
        .owner_color_source
        .as_deref()
        .map(|value| format!(" / owner_color={value}"))
        .unwrap_or_default();
    let source_triangle = label
        .source_triangle
        .filter(|source_triangle| Some(*source_triangle) != label.triangle)
        .map(|value| format!(" / src_tri{value}"))
        .unwrap_or_default();
    format!(
        "{} / {} / tri{}{} / material={} / draw{}{}{}",
        label.pass.as_deref().unwrap_or("unknown"),
        label.mesh_name.as_deref().unwrap_or("unknown"),
        label
            .triangle
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_owned()),
        source_triangle,
        label.material_name.as_deref().unwrap_or("unknown-material"),
        label
            .draw_index
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_owned()),
        bevy_phase,
        owner_color,
    )
}

fn pixels_cell(pixels: &[Value]) -> String {
    pixels
        .iter()
        .filter_map(|pixel| {
            pixel
                .get("x")
                .and_then(Value::as_u64)
                .zip(pixel.get("y").and_then(Value::as_u64))
                .map(|(x, y)| format!("({x},{y})"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_count(output: &mut String, label: &str, value: Option<u64>) {
    if let Some(value) = value {
        output.push_str(&format!("| `{label}` | {value} |\n"));
    }
}

fn write_projection_gap_count(output: &mut String, report: &OwnerTailReport, label: &str) {
    write_count(
        output,
        label,
        report
            .unexplained_projection_gap_summary
            .get(label)
            .and_then(Value::as_u64),
    );
}

fn write_projection_gap_value(output: &mut String, report: &OwnerTailReport, label: &str) {
    if let Some(value) = report
        .unexplained_projection_gap_summary
        .get(label)
        .and_then(Value::as_f64)
    {
        output.push_str(&format!("| `{label}` | {value:.8} |\n"));
    }
}

fn write_raster_bounds_summary(output: &mut String, image: &str, summary: &Value) {
    for key in [
        "owners_with_pixels",
        "owners_with_screen_bounds",
        "owners_with_center_bounds_excess",
        "owners_with_origin_bounds_excess",
        "pixels_with_screen_bounds",
        "pixels_in_center_bounds_excess_owners",
        "pixels_in_origin_bounds_excess_owners",
    ] {
        if let Some(value) = summary.get(key).and_then(Value::as_u64) {
            output.push_str(&format!("| `{image}` | `{key}` | {value} |\n"));
        }
    }
    for key in ["max_center_bounds_excess", "max_origin_bounds_excess"] {
        if let Some(value) = summary.get(key).and_then(Value::as_f64) {
            output.push_str(&format!("| `{image}` | `{key}` | {value:.8} |\n"));
        }
    }
}

fn write_raster_alignment_summary(output: &mut String, image: &str, summary: &Value) {
    for key in [
        "owners_with_pixels",
        "owners_with_screen_bounds",
        "owners_aligned_to_self",
        "owners_aligned_elsewhere",
        "pixels_aligned_elsewhere",
        "owners_aligned_elsewhere_over_2px",
        "owners_aligned_elsewhere_over_4px",
        "pixels_aligned_elsewhere_over_2px",
        "pixels_aligned_elsewhere_over_4px",
    ] {
        if let Some(value) = summary.get(key).and_then(Value::as_u64) {
            output.push_str(&format!("| `{image}` | `{key}` | {value} |\n"));
        }
    }
    if let Some(value) = summary
        .get("max_self_center_distance")
        .and_then(Value::as_f64)
    {
        output.push_str(&format!(
            "| `{image}` | `max_self_center_distance` | {value:.8} |\n"
        ));
    }
}

fn markdown_text_field(value: &Value, key: &str) -> String {
    text_field(value, key)
}

fn text_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .replace('|', "\\|")
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn u64_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn i64_field(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

fn optional_i64_cell(value: &Value, key: &str) -> String {
    i64_field(value, key)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_owned())
}

fn f64_field(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn bool_field(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn display_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn self_test() -> Result<(), Box<dyn std::error::Error>> {
    let value = serde_json::from_str::<Value>(
        r#"{
        "expected": "reference.imqraw",
        "actual": "candidate.imqraw",
        "width": 2,
        "height": 2,
        "mismatched_shared_nonzero": 2,
        "same_projected_triangle_mismatched_shared_nonzero": 0,
        "same_projected_or_touching_triangle_mismatched_shared_nonzero": 1,
        "unexplained_owner_tail_mismatched_shared_nonzero": 1,
        "unexplained_owner_tail_after_touching_mismatched_shared_nonzero": 1,
        "actual_not_visible_by_cull_policy_mismatched_shared_nonzero": 1,
        "actual_metadata_bounds_miss_mismatched_shared_nonzero": 1,
        "actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero": 1,
        "unexplained_projection_gap_summary": {
            "with_screen_bounds": 2,
            "overlapping_screen_bounds_1px": 1,
            "disjoint_screen_bounds_1px": 1,
            "pixel_near_either_edge_025px": 0,
            "pixel_near_both_edges_025px": 0,
            "pixel_near_either_edge_05px": 2,
            "pixel_near_both_edges_05px": 1,
            "pixel_inside_expected_screen_bounds": 1,
            "pixel_inside_actual_screen_bounds": 2,
            "pixel_inside_both_screen_bounds": 1,
            "pixel_inside_expected_only_screen_bounds": 0,
            "pixel_inside_actual_only_screen_bounds": 1,
            "pixel_inside_neither_screen_bounds": 0,
            "with_screen_triangles": 2,
            "pixel_center_inside_expected_screen_triangle": 1,
            "pixel_center_inside_actual_screen_triangle": 1,
            "pixel_center_inside_both_screen_triangles": 0,
            "pixel_center_inside_expected_only_screen_triangle": 1,
            "pixel_center_inside_actual_only_screen_triangle": 1,
            "pixel_center_inside_neither_screen_triangle": 0,
            "pixel_origin_inside_expected_screen_triangle": 1,
            "pixel_origin_inside_actual_screen_triangle": 1,
            "pixel_origin_inside_both_screen_triangles": 0,
            "pixel_origin_inside_expected_only_screen_triangle": 1,
            "pixel_origin_inside_actual_only_screen_triangle": 1,
            "pixel_origin_inside_neither_screen_triangle": 0,
            "pixel_subpixel3_inside_expected_screen_triangle": 1,
            "pixel_subpixel3_inside_actual_screen_triangle": 2,
            "pixel_subpixel3_inside_both_screen_triangles": 1,
            "pixel_subpixel3_inside_expected_only_screen_triangle": 0,
            "pixel_subpixel3_inside_actual_only_screen_triangle": 1,
            "pixel_subpixel3_inside_neither_screen_triangle": 0,
            "pixel_near_expected_min_x_edge_05px": 0,
            "pixel_near_expected_max_x_edge_05px": 1,
            "pixel_near_expected_min_y_edge_05px": 1,
            "pixel_near_expected_max_y_edge_05px": 0,
            "pixel_near_actual_min_x_edge_05px": 2,
            "pixel_near_actual_max_x_edge_05px": 0,
            "pixel_near_actual_min_y_edge_05px": 2,
            "pixel_near_actual_max_y_edge_05px": 0,
            "either_small_bounds_area_le_1px": 0,
            "both_small_bounds_area_le_1px": 0,
            "either_small_bounds_area_le_4px": 2,
            "both_small_bounds_area_le_4px": 2,
            "with_actual_bevy_phase_order_offset_applied": 1,
            "actual_bevy_phase_order_offset_applied_nonzero": 1,
            "mean_actual_bevy_phase_order_offset_applied": 0.000019,
            "max_actual_bevy_phase_order_offset_applied": 0.000019
        },
        "top_unexplained_material_transitions": [{
            "expected_pass": "outline",
            "expected_mesh": "wear_4",
            "expected_material": "huku_bake (Outline)",
            "actual_pass": "base",
            "actual_mesh": "wear",
            "actual_material": "huku_bake",
            "material_relation": "same-index",
            "triangle_relation": "different-triangle",
            "projection_relation": "overlap-depth-close",
            "count": 1
        }],
        "top_actual_metadata_recoveries": [{
            "decoded_actual": 34459,
            "recovered_actual": 34715,
            "id_delta": 256,
            "red_delta": 0,
            "green_delta": 1,
            "blue_delta": 0,
            "channel_manhattan_delta": 1,
            "channel_chebyshev_delta": 1,
            "channel_delta_class": "g+1",
            "decoded_mesh_name": "wear",
            "recovered_mesh_name": "wear",
            "decoded_material_name": "huku_bake",
            "recovered_material_name": "huku_bake",
            "decoded_triangle": 24,
            "recovered_triangle": 25,
            "decoded_source_triangle": 24,
            "recovered_source_triangle": 25,
            "decoded_draw_index": 100,
            "recovered_draw_index": 101,
            "source_triangle_delta": 1,
            "draw_index_delta": 1,
            "mesh_relation": "same-normalized-name",
            "material_relation": "same-name",
            "triangle_relation": "adjacent-triangle-index",
            "projection_relation": "overlap-depth-close",
            "count": 34
        }],
        "top_unexplained_expected_to_actual_details": [{
            "count": 1,
            "expected": {
                "id": 10,
                "pass": "outline",
                "mesh_name": "wear_4",
                "material_name": "huku_bake (Outline)",
                "material_index": 5,
                "triangle": 42,
                "indices": [1, 2, 3],
                "render_order": 19,
                "render_phase_order": 19,
                "draw_index": 10,
                "webgl_depth": 0.5
            },
            "actual": {
                "id": 11,
                "pass": "base",
                "mesh_name": "wear",
                "material_name": "huku_bake",
                "material_index": 5,
                "triangle": 50,
                "indices": [4, 5, 6],
                "render_order": 2000,
                "render_phase_order": 19,
                "bevy_phase_order_offset": 0.000019,
                "bevy_phase_order_offset_applied": 0.000019,
                "draw_index": 11,
                "webgl_depth": 0.49
            },
            "sample_pixels": [{"x": 1, "y": 1}]
        }]
    }"#,
    )?;
    let report = summarize_report(Path::new("owner.json"), &value, 16)?;
    assert_eq!(
        report
            .counts
            .unexplained_owner_tail_after_touching_mismatched_shared_nonzero,
        Some(1)
    );
    assert_eq!(report.top_unexplained_details.len(), 1);
    assert_eq!(
        report.top_unexplained_details[0].relation.material,
        "same-index"
    );
    let markdown = markdown_report(&report);
    assert!(markdown.contains("huku_bake (Outline)"));
    assert!(markdown.contains("depth_delta=0.010000"));
    assert!(markdown.contains("actual_not_visible_by_cull_policy_mismatched_shared_nonzero"));
    assert!(
        markdown
            .contains("actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero")
    );
    assert!(markdown.contains("Projection Gap Shape"));
    assert!(markdown.contains("Top Actual Metadata Recoveries"));
    assert!(markdown.contains(
        "| 34 | 34459 | 34715 | 256 | (+0, +1, +0) | g+1 | 1 | 1 | same-normalized-name; same-name; adjacent-triangle-index; overlap-depth-close |"
    ));
    assert!(markdown.contains("pixel_near_either_edge_05px"));
    assert!(markdown.contains("pixel_inside_both_screen_bounds"));
    assert!(markdown.contains("pixel_subpixel3_inside_both_screen_triangles"));
    assert!(markdown.contains("pixel_subpixel3_inside_actual_only_screen_triangle"));
    assert!(markdown.contains("pixel_near_actual_min_y_edge_05px"));
    assert!(markdown.contains("either_small_bounds_area_le_4px"));
    assert!(markdown.contains("actual_bevy_phase_order_offset_applied_nonzero"));
    assert!(markdown.contains("bevy_phase=0.00001900"));
    Ok(())
}
