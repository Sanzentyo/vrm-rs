#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
imq = { git = "https://github.com/Sanzentyo/imq.git", rev = "0fdc5263c0c21bd6d7bc55c194e98b593bf83bff", default-features = false }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
---

//! Compare two `owner-id` diagnostic imqraw images.

use clap::Parser;
use imq::{PixelFormat, RawImageRecord, decode_imqraw_bundle};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "compare-owner-id-images",
    about = "Compare two owner-id diagnostic imqraw artifacts"
)]
struct Options {
    #[arg(long)]
    expected: Option<PathBuf>,
    #[arg(long)]
    actual: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long, default_value_t = 0)]
    expected_index: usize,
    #[arg(long, default_value_t = 0)]
    actual_index: usize,
    #[arg(long, default_value_t = 12)]
    top: usize,
    #[arg(long)]
    self_test: bool,
}

#[derive(Clone, Debug)]
struct RgbaImage {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerCompareReport {
    expected: String,
    actual: String,
    width: usize,
    height: usize,
    pixels: usize,
    expected_nonzero: u64,
    actual_nonzero: u64,
    shared_nonzero: u64,
    exact_owner_matches: u64,
    expected_found_in_actual_neighborhood_1px: u64,
    actual_found_in_expected_neighborhood_1px: u64,
    expected_only: u64,
    actual_only: u64,
    mismatched_shared_nonzero: u64,
    same_projected_triangle_mismatched_shared_nonzero: u64,
    same_projected_or_adjacent_triangle_mismatched_shared_nonzero: u64,
    actual_not_visible_by_cull_policy_shared_nonzero: u64,
    actual_not_visible_by_cull_policy_mismatched_shared_nonzero: u64,
    actual_metadata_bounds_miss_shared_nonzero: u64,
    actual_metadata_bounds_miss_mismatched_shared_nonzero: u64,
    actual_metadata_bounds_miss_recovered_by_near_id_shared_nonzero: u64,
    actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero: u64,
    exact_owner_match_ratio: f64,
    expected_neighborhood_1px_ratio: f64,
    actual_neighborhood_1px_ratio: f64,
    max_expected_owner: u32,
    max_actual_owner: u32,
    top_owner_id_deltas: Vec<OwnerIdDelta>,
    top_pass_transitions: Vec<OwnerPassTransition>,
    top_owner_geometry_classes: Vec<OwnerGeometryClassTransition>,
    top_render_policy_transitions: Vec<OwnerRenderPolicyTransition>,
    top_actual_cull_visibility: Vec<OwnerActualCullVisibility>,
    top_actual_metadata_recoveries: Vec<OwnerMetadataRecovery>,
    top_expected_to_actual: Vec<OwnerTransition>,
    top_actual_to_expected: Vec<OwnerTransition>,
    top_expected_to_actual_details: Vec<OwnerTransitionDetail>,
    top_actual_to_expected_details: Vec<OwnerTransitionDetail>,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerTransition {
    expected: u32,
    actual: u32,
    count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerIdDelta {
    expected_minus_actual: i64,
    count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerPassTransition {
    expected_pass: String,
    actual_pass: String,
    count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerGeometryClassTransition {
    pass_relation: String,
    mesh_relation: String,
    material_relation: String,
    triangle_relation: String,
    projection_relation: String,
    count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerRenderPolicyTransition {
    expected_pass: String,
    expected_side: String,
    expected_cull_mode: String,
    expected_front_face: String,
    expected_front_facing: String,
    expected_gpu_front_facing: String,
    expected_visible_by_cull_policy: String,
    expected_depth_write: String,
    actual_pass: String,
    actual_cull_mode: String,
    actual_front_face: String,
    actual_front_facing: String,
    actual_gpu_front_facing: String,
    actual_visible_by_cull_policy: String,
    actual_depth_write: String,
    count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerActualCullVisibility {
    actual_pass: String,
    actual_cull_mode: String,
    actual_front_face: String,
    actual_front_facing: String,
    actual_gpu_front_facing: String,
    actual_visible_by_cull_policy: String,
    actual_depth_write: String,
    count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerMetadataRecovery {
    decoded_actual: u32,
    recovered_actual: u32,
    count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct OwnerRenderPolicyKey {
    expected_pass: String,
    expected_side: String,
    expected_cull_mode: String,
    expected_front_face: String,
    expected_front_facing: String,
    expected_gpu_front_facing: String,
    expected_visible_by_cull_policy: String,
    expected_depth_write: String,
    actual_pass: String,
    actual_cull_mode: String,
    actual_front_face: String,
    actual_front_facing: String,
    actual_gpu_front_facing: String,
    actual_visible_by_cull_policy: String,
    actual_depth_write: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct OwnerActualCullVisibilityKey {
    actual_pass: String,
    actual_cull_mode: String,
    actual_front_face: String,
    actual_front_facing: String,
    actual_gpu_front_facing: String,
    actual_visible_by_cull_policy: String,
    actual_depth_write: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct OwnerGeometryClassKey {
    pass_relation: String,
    mesh_relation: String,
    material_relation: String,
    triangle_relation: String,
    projection_relation: String,
}

#[derive(Clone, Debug, Default, Serialize)]
struct OwnerLabel {
    id: u32,
    material_name: Option<String>,
    pass: Option<String>,
    node_name: Option<String>,
    mesh_name: Option<String>,
    node_index: Option<u64>,
    mesh_index: Option<u64>,
    primitive_index: Option<u64>,
    material_index: Option<i64>,
    material_slot: Option<u64>,
    triangle: Option<u64>,
    render_order: Option<i64>,
    draw_index: Option<u64>,
    material_type: Option<String>,
    front_face: Option<String>,
    cull_mode: Option<String>,
    material_side: Option<i64>,
    alpha_mode: Option<String>,
    alpha_test: Option<f64>,
    opacity: Option<f64>,
    transparent: Option<bool>,
    depth_write: Option<bool>,
    depth_test: Option<bool>,
    depth_compare: Option<String>,
    blend: Option<bool>,
    blending: Option<i64>,
    premultiplied_alpha: Option<bool>,
    alpha_cutoff: Option<f64>,
    depth_bias: Option<f64>,
    screen_bounds: Option<OwnerScreenBounds>,
    depth: Option<f64>,
    webgl_depth: Option<f64>,
    depth_range: Option<String>,
    screen_signed_area: Option<f64>,
    front_facing: Option<bool>,
    gpu_front_facing: Option<bool>,
    visible_by_cull_policy: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerTransitionDetail {
    expected: OwnerLabel,
    actual: OwnerLabel,
    count: u64,
    bounds: Option<OwnerPixelBounds>,
    sample_pixels: Vec<OwnerPixel>,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct OwnerPixel {
    x: usize,
    y: usize,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct OwnerPixelBounds {
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct OwnerScreenBounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

#[derive(Clone, Debug)]
struct OwnerTransitionPixels {
    bounds: OwnerPixelBounds,
    sample_pixels: Vec<OwnerPixel>,
}

fn main() {
    if let Err(error) = run(Options::parse()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    if options.self_test {
        self_test()?;
        return Ok(());
    }

    let expected_path = options
        .expected
        .as_deref()
        .ok_or("--expected is required unless --self-test is set")?;
    let actual_path = options
        .actual
        .as_deref()
        .ok_or("--actual is required unless --self-test is set")?;
    let expected = read_imqraw_rgba8(expected_path, options.expected_index)?;
    let actual = read_imqraw_rgba8(actual_path, options.actual_index)?;
    let expected_metadata = read_owner_metadata_for_imqraw(expected_path).unwrap_or_default();
    let actual_metadata = read_owner_metadata_for_imqraw(actual_path).unwrap_or_default();
    let report = compare_owner_images(
        display_path(expected_path),
        display_path(actual_path),
        &expected,
        &actual,
        &expected_metadata,
        &actual_metadata,
        options.top,
    )?;
    let json = format!("{}\n", serde_json::to_string_pretty(&report)?);
    if let Some(path) = &options.out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)?;
    } else {
        print!("{json}");
    }
    Ok(())
}

fn compare_owner_images(
    expected_name: String,
    actual_name: String,
    expected: &RgbaImage,
    actual: &RgbaImage,
    expected_metadata: &HashMap<u32, OwnerLabel>,
    actual_metadata: &HashMap<u32, OwnerLabel>,
    top: usize,
) -> Result<OwnerCompareReport, Box<dyn Error>> {
    if expected.width != actual.width || expected.height != actual.height {
        return Err(format!(
            "image dimensions differ: expected {}x{}, actual {}x{}",
            expected.width, expected.height, actual.width, actual.height
        )
        .into());
    }

    let expected_ids = owner_ids(expected);
    let actual_ids = owner_ids(actual);
    let mut expected_nonzero = 0;
    let mut actual_nonzero = 0;
    let mut shared_nonzero = 0;
    let mut exact_owner_matches = 0;
    let mut expected_found_in_actual_neighborhood_1px = 0;
    let mut actual_found_in_expected_neighborhood_1px = 0;
    let mut expected_only = 0;
    let mut actual_only = 0;
    let mut mismatched_shared_nonzero = 0;
    let mut same_projected_triangle_mismatched_shared_nonzero = 0;
    let mut same_projected_or_adjacent_triangle_mismatched_shared_nonzero = 0;
    let mut actual_not_visible_by_cull_policy_shared_nonzero = 0;
    let mut actual_not_visible_by_cull_policy_mismatched_shared_nonzero = 0;
    let mut actual_metadata_bounds_miss_shared_nonzero = 0;
    let mut actual_metadata_bounds_miss_mismatched_shared_nonzero = 0;
    let mut actual_metadata_bounds_miss_recovered_by_near_id_shared_nonzero = 0;
    let mut actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero = 0;
    let mut expected_to_actual = BTreeMap::new();
    let mut actual_to_expected = BTreeMap::new();
    let mut owner_id_deltas = BTreeMap::new();
    let mut pass_transitions = BTreeMap::new();
    let mut owner_geometry_classes = BTreeMap::new();
    let mut render_policy_transitions = BTreeMap::new();
    let mut actual_cull_visibility = BTreeMap::new();
    let mut actual_metadata_recoveries = BTreeMap::new();
    let mut expected_to_actual_pixels = BTreeMap::new();
    let mut actual_to_expected_pixels = BTreeMap::new();

    for (index, (&expected_id, &actual_id)) in expected_ids.iter().zip(&actual_ids).enumerate() {
        let pixel = OwnerPixel {
            x: index % expected.width,
            y: index / expected.width,
        };
        if expected_id != 0 {
            expected_nonzero += 1;
        }
        if actual_id != 0 {
            actual_nonzero += 1;
        }
        match (expected_id, actual_id) {
            (0, 0) => {}
            (0, _) => actual_only += 1,
            (_, 0) => expected_only += 1,
            (left, right) => {
                shared_nonzero += 1;
                let actual_label = actual_metadata.get(&right);
                let actual_is_culled = actual_label
                    .and_then(|label| label.visible_by_cull_policy)
                    .is_some_and(|visible| !visible);
                if actual_is_culled {
                    actual_not_visible_by_cull_policy_shared_nonzero += 1;
                }
                let actual_metadata_bounds_miss =
                    actual_label.is_some_and(|label| !owner_label_contains_pixel(label, pixel, 2.0));
                let actual_metadata_recovery = actual_metadata_bounds_miss
                    .then(|| recover_near_actual_owner(right, pixel, actual_metadata))
                    .flatten();
                if actual_metadata_bounds_miss {
                    actual_metadata_bounds_miss_shared_nonzero += 1;
                    if let Some(recovered) = actual_metadata_recovery {
                        actual_metadata_bounds_miss_recovered_by_near_id_shared_nonzero += 1;
                        *actual_metadata_recoveries
                            .entry((right, recovered))
                            .or_default() += 1;
                    }
                }
                bump_actual_cull_visibility(&mut actual_cull_visibility, actual_label);
                if left == right {
                    exact_owner_matches += 1;
                } else {
                    mismatched_shared_nonzero += 1;
                    if actual_is_culled {
                        actual_not_visible_by_cull_policy_mismatched_shared_nonzero += 1;
                    }
                    if actual_metadata_bounds_miss {
                        actual_metadata_bounds_miss_mismatched_shared_nonzero += 1;
                        if actual_metadata_recovery.is_some() {
                            actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero += 1;
                        }
                    }
                    let expected_label = expected_metadata.get(&left);
                    let geometry_class =
                        OwnerGeometryClassKey::from_labels(expected_label, actual_label);
                    if geometry_class.is_same_projected_triangle() {
                        same_projected_triangle_mismatched_shared_nonzero += 1;
                    }
                    if geometry_class.is_same_projected_or_adjacent_triangle() {
                        same_projected_or_adjacent_triangle_mismatched_shared_nonzero += 1;
                    }
                    bump_transition(&mut expected_to_actual, left, right);
                    bump_transition(&mut actual_to_expected, right, left);
                    bump_transition_pixels(&mut expected_to_actual_pixels, left, right, pixel);
                    bump_transition_pixels(&mut actual_to_expected_pixels, right, left, pixel);
                    bump_pass_transition(
                        &mut pass_transitions,
                        expected_metadata.get(&left),
                        actual_label,
                    );
                    bump_owner_geometry_class(
                        &mut owner_geometry_classes,
                        expected_label,
                        actual_label,
                    );
                    bump_render_policy_transition(
                        &mut render_policy_transitions,
                        expected_metadata.get(&left),
                        actual_label,
                    );
                    *owner_id_deltas
                        .entry(i64::from(left) - i64::from(right))
                        .or_default() += 1;
                }
                if neighborhood_contains(
                    &actual_ids,
                    expected.width,
                    expected.height,
                    index,
                    left,
                    1,
                ) {
                    expected_found_in_actual_neighborhood_1px += 1;
                }
                if neighborhood_contains(
                    &expected_ids,
                    expected.width,
                    expected.height,
                    index,
                    right,
                    1,
                ) {
                    actual_found_in_expected_neighborhood_1px += 1;
                }
            }
        }
    }

    Ok(OwnerCompareReport {
        expected: expected_name,
        actual: actual_name,
        width: expected.width,
        height: expected.height,
        pixels: expected_ids.len(),
        expected_nonzero,
        actual_nonzero,
        shared_nonzero,
        exact_owner_matches,
        expected_found_in_actual_neighborhood_1px,
        actual_found_in_expected_neighborhood_1px,
        expected_only,
        actual_only,
        mismatched_shared_nonzero,
        same_projected_triangle_mismatched_shared_nonzero,
        same_projected_or_adjacent_triangle_mismatched_shared_nonzero,
        actual_not_visible_by_cull_policy_shared_nonzero,
        actual_not_visible_by_cull_policy_mismatched_shared_nonzero,
        actual_metadata_bounds_miss_shared_nonzero,
        actual_metadata_bounds_miss_mismatched_shared_nonzero,
        actual_metadata_bounds_miss_recovered_by_near_id_shared_nonzero,
        actual_metadata_bounds_miss_recovered_by_near_id_mismatched_shared_nonzero,
        exact_owner_match_ratio: ratio(exact_owner_matches, shared_nonzero),
        expected_neighborhood_1px_ratio: ratio(
            expected_found_in_actual_neighborhood_1px,
            shared_nonzero,
        ),
        actual_neighborhood_1px_ratio: ratio(
            actual_found_in_expected_neighborhood_1px,
            shared_nonzero,
        ),
        max_expected_owner: expected_ids.iter().copied().max().unwrap_or(0),
        max_actual_owner: actual_ids.iter().copied().max().unwrap_or(0),
        top_owner_id_deltas: top_deltas(owner_id_deltas, top),
        top_pass_transitions: top_pass_transitions(pass_transitions, top),
        top_owner_geometry_classes: top_owner_geometry_classes(owner_geometry_classes, top),
        top_render_policy_transitions: top_render_policy_transitions(
            render_policy_transitions,
            top,
        ),
        top_actual_cull_visibility: top_actual_cull_visibility(actual_cull_visibility, top),
        top_actual_metadata_recoveries: top_actual_metadata_recoveries(
            actual_metadata_recoveries,
            top,
        ),
        top_expected_to_actual: top_transitions(expected_to_actual.clone(), top),
        top_actual_to_expected: top_transitions(actual_to_expected.clone(), top),
        top_expected_to_actual_details: top_transition_details(
            &expected_to_actual,
            &expected_to_actual_pixels,
            expected_metadata,
            actual_metadata,
            top,
        ),
        top_actual_to_expected_details: top_transition_details(
            &actual_to_expected,
            &actual_to_expected_pixels,
            actual_metadata,
            expected_metadata,
            top,
        ),
    })
}

fn owner_ids(image: &RgbaImage) -> Vec<u32> {
    image
        .rgba
        .chunks_exact(4)
        .map(|rgba| u32::from(rgba[0]) | (u32::from(rgba[1]) << 8) | (u32::from(rgba[2]) << 16))
        .collect()
}

fn top_deltas(map: BTreeMap<i64, u64>, top: usize) -> Vec<OwnerIdDelta> {
    let mut entries = map.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    entries
        .into_iter()
        .take(top)
        .map(|(expected_minus_actual, count)| OwnerIdDelta {
            expected_minus_actual,
            count,
        })
        .collect()
}

fn bump_pass_transition(
    map: &mut BTreeMap<(String, String), u64>,
    expected: Option<&OwnerLabel>,
    actual: Option<&OwnerLabel>,
) {
    let expected_pass = expected
        .and_then(|label| label.pass.as_deref())
        .unwrap_or("unknown")
        .to_owned();
    let actual_pass = actual
        .and_then(|label| label.pass.as_deref())
        .unwrap_or("unknown")
        .to_owned();
    *map.entry((expected_pass, actual_pass)).or_default() += 1;
}

fn top_pass_transitions(
    map: BTreeMap<(String, String), u64>,
    top: usize,
) -> Vec<OwnerPassTransition> {
    let mut entries = map.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    entries
        .into_iter()
        .take(top)
        .map(|((expected_pass, actual_pass), count)| OwnerPassTransition {
            expected_pass,
            actual_pass,
            count,
        })
        .collect()
}

fn bump_owner_geometry_class(
    map: &mut BTreeMap<OwnerGeometryClassKey, u64>,
    expected: Option<&OwnerLabel>,
    actual: Option<&OwnerLabel>,
) {
    *map.entry(OwnerGeometryClassKey::from_labels(expected, actual))
        .or_default() += 1;
}

fn top_owner_geometry_classes(
    map: BTreeMap<OwnerGeometryClassKey, u64>,
    top: usize,
) -> Vec<OwnerGeometryClassTransition> {
    let mut entries = map.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    entries
        .into_iter()
        .take(top)
        .map(|(key, count)| OwnerGeometryClassTransition {
            pass_relation: key.pass_relation,
            mesh_relation: key.mesh_relation,
            material_relation: key.material_relation,
            triangle_relation: key.triangle_relation,
            projection_relation: key.projection_relation,
            count,
        })
        .collect()
}

fn bump_render_policy_transition(
    map: &mut BTreeMap<OwnerRenderPolicyKey, u64>,
    expected: Option<&OwnerLabel>,
    actual: Option<&OwnerLabel>,
) {
    *map.entry(OwnerRenderPolicyKey::from_labels(expected, actual))
        .or_default() += 1;
}

fn top_render_policy_transitions(
    map: BTreeMap<OwnerRenderPolicyKey, u64>,
    top: usize,
) -> Vec<OwnerRenderPolicyTransition> {
    let mut entries = map.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    entries
        .into_iter()
        .take(top)
        .map(|(key, count)| OwnerRenderPolicyTransition {
            expected_pass: key.expected_pass,
            expected_side: key.expected_side,
            expected_cull_mode: key.expected_cull_mode,
            expected_front_face: key.expected_front_face,
            expected_front_facing: key.expected_front_facing,
            expected_gpu_front_facing: key.expected_gpu_front_facing,
            expected_visible_by_cull_policy: key.expected_visible_by_cull_policy,
            expected_depth_write: key.expected_depth_write,
            actual_pass: key.actual_pass,
            actual_cull_mode: key.actual_cull_mode,
            actual_front_face: key.actual_front_face,
            actual_front_facing: key.actual_front_facing,
            actual_gpu_front_facing: key.actual_gpu_front_facing,
            actual_visible_by_cull_policy: key.actual_visible_by_cull_policy,
            actual_depth_write: key.actual_depth_write,
            count,
        })
        .collect()
}

fn bump_actual_cull_visibility(
    map: &mut BTreeMap<OwnerActualCullVisibilityKey, u64>,
    actual: Option<&OwnerLabel>,
) {
    *map.entry(OwnerActualCullVisibilityKey::from_label(actual))
        .or_default() += 1;
}

fn top_actual_cull_visibility(
    map: BTreeMap<OwnerActualCullVisibilityKey, u64>,
    top: usize,
) -> Vec<OwnerActualCullVisibility> {
    let mut entries = map.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    entries
        .into_iter()
        .take(top)
        .map(|(key, count)| OwnerActualCullVisibility {
            actual_pass: key.actual_pass,
            actual_cull_mode: key.actual_cull_mode,
            actual_front_face: key.actual_front_face,
            actual_front_facing: key.actual_front_facing,
            actual_gpu_front_facing: key.actual_gpu_front_facing,
            actual_visible_by_cull_policy: key.actual_visible_by_cull_policy,
            actual_depth_write: key.actual_depth_write,
            count,
        })
        .collect()
}

fn top_actual_metadata_recoveries(
    map: BTreeMap<(u32, u32), u64>,
    top: usize,
) -> Vec<OwnerMetadataRecovery> {
    let mut entries = map.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.0.cmp(&right.0.0))
            .then_with(|| left.0.1.cmp(&right.0.1))
    });
    entries
        .into_iter()
        .take(top)
        .map(|((decoded_actual, recovered_actual), count)| OwnerMetadataRecovery {
            decoded_actual,
            recovered_actual,
            count,
        })
        .collect()
}

fn owner_label_contains_pixel(label: &OwnerLabel, pixel: OwnerPixel, pad: f64) -> bool {
    label.screen_bounds.is_some_and(|bounds| {
        let x = pixel.x as f64;
        let y = pixel.y as f64;
        x >= bounds.min_x - pad
            && x <= bounds.max_x + pad
            && y >= bounds.min_y - pad
            && y <= bounds.max_y + pad
    })
}

fn recover_near_actual_owner(
    decoded_actual: u32,
    pixel: OwnerPixel,
    metadata: &HashMap<u32, OwnerLabel>,
) -> Option<u32> {
    near_owner_id_candidates(decoded_actual)
        .into_iter()
        .filter_map(|candidate| {
            metadata
                .get(&candidate)
                .filter(|label| owner_label_contains_pixel(label, pixel, 2.0))
                .map(|_| candidate)
        })
        .min_by_key(|candidate| candidate.abs_diff(decoded_actual))
}

fn near_owner_id_candidates(id: u32) -> Vec<u32> {
    let mut candidates = Vec::new();
    for db in -1_i32..=1 {
        for dg in -1_i32..=1 {
            for dr in -2_i32..=2 {
                if dr == 0 && dg == 0 && db == 0 {
                    continue;
                }
                let delta = dr + dg * 256 + db * 65_536;
                let candidate = if delta < 0 {
                    id.checked_sub(delta.unsigned_abs())
                } else {
                    id.checked_add(delta as u32)
                };
                if let Some(candidate) = candidate.filter(|candidate| *candidate != 0) {
                    candidates.push(candidate);
                }
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates
}

impl OwnerRenderPolicyKey {
    fn from_labels(expected: Option<&OwnerLabel>, actual: Option<&OwnerLabel>) -> Self {
        Self {
            expected_pass: pass_label(expected),
            expected_side: expected
                .and_then(|label| label.material_side)
                .map(material_side_label)
                .unwrap_or_else(|| "unknown".to_owned()),
            expected_cull_mode: expected
                .and_then(|label| label.cull_mode.as_deref())
                .unwrap_or("unknown")
                .to_owned(),
            expected_front_face: expected
                .and_then(|label| label.front_face.as_deref())
                .unwrap_or("unknown")
                .to_owned(),
            expected_front_facing: optional_bool_label(expected.and_then(|label| label.front_facing)),
            expected_gpu_front_facing: optional_bool_label(
                expected.and_then(|label| label.gpu_front_facing),
            ),
            expected_visible_by_cull_policy: optional_bool_label(
                expected.and_then(|label| label.visible_by_cull_policy),
            ),
            expected_depth_write: optional_bool_label(expected.and_then(|label| label.depth_write)),
            actual_pass: pass_label(actual),
            actual_cull_mode: actual
                .and_then(|label| label.cull_mode.as_deref())
                .unwrap_or("unknown")
                .to_owned(),
            actual_front_face: actual
                .and_then(|label| label.front_face.as_deref())
                .unwrap_or("unknown")
                .to_owned(),
            actual_front_facing: optional_bool_label(actual.and_then(|label| label.front_facing)),
            actual_gpu_front_facing: optional_bool_label(
                actual.and_then(|label| label.gpu_front_facing),
            ),
            actual_visible_by_cull_policy: optional_bool_label(
                actual.and_then(|label| label.visible_by_cull_policy),
            ),
            actual_depth_write: optional_bool_label(actual.and_then(|label| label.depth_write)),
        }
    }
}

impl OwnerActualCullVisibilityKey {
    fn from_label(actual: Option<&OwnerLabel>) -> Self {
        Self {
            actual_pass: pass_label(actual),
            actual_cull_mode: actual
                .and_then(|label| label.cull_mode.as_deref())
                .unwrap_or("unknown")
                .to_owned(),
            actual_front_face: actual
                .and_then(|label| label.front_face.as_deref())
                .unwrap_or("unknown")
                .to_owned(),
            actual_front_facing: optional_bool_label(actual.and_then(|label| label.front_facing)),
            actual_gpu_front_facing: optional_bool_label(
                actual.and_then(|label| label.gpu_front_facing),
            ),
            actual_visible_by_cull_policy: optional_bool_label(
                actual.and_then(|label| label.visible_by_cull_policy),
            ),
            actual_depth_write: optional_bool_label(actual.and_then(|label| label.depth_write)),
        }
    }
}

impl OwnerGeometryClassKey {
    fn from_labels(expected: Option<&OwnerLabel>, actual: Option<&OwnerLabel>) -> Self {
        Self {
            pass_relation: relation_label(
                expected.and_then(|label| label.pass.as_deref()),
                actual.and_then(|label| label.pass.as_deref()),
            ),
            mesh_relation: mesh_relation(expected, actual),
            material_relation: material_relation(expected, actual),
            triangle_relation: triangle_relation(expected, actual),
            projection_relation: projection_relation(expected, actual),
        }
    }

    fn is_same_projected_triangle(&self) -> bool {
        self.pass_relation == "same"
            && self.mesh_relation != "different"
            && self.triangle_relation == "same-triangle"
            && self.projection_relation == "overlap-depth-close"
    }

    fn is_same_projected_or_adjacent_triangle(&self) -> bool {
        self.pass_relation == "same"
            && self.mesh_relation != "different"
            && matches!(
                self.triangle_relation.as_str(),
                "same-triangle" | "adjacent-triangle-index"
            )
            && self.projection_relation == "overlap-depth-close"
    }
}

fn relation_label(left: Option<&str>, right: Option<&str>) -> String {
    match (left, right) {
        (Some(left), Some(right)) if left == right => "same".to_owned(),
        (Some(_), Some(_)) => "different".to_owned(),
        _ => "unknown".to_owned(),
    }
}

fn mesh_relation(expected: Option<&OwnerLabel>, actual: Option<&OwnerLabel>) -> String {
    match (expected, actual) {
        (Some(expected), Some(actual)) => {
            if same_optional_u64(expected.mesh_index, actual.mesh_index) {
                "same-index".to_owned()
            } else if same_optional_str(expected.mesh_name.as_deref(), actual.mesh_name.as_deref())
            {
                "same-name".to_owned()
            } else if normalized_mesh_name(expected.mesh_name.as_deref())
                == normalized_mesh_name(actual.mesh_name.as_deref())
            {
                "same-normalized-name".to_owned()
            } else {
                "different".to_owned()
            }
        }
        _ => "unknown".to_owned(),
    }
}

fn material_relation(expected: Option<&OwnerLabel>, actual: Option<&OwnerLabel>) -> String {
    match (expected, actual) {
        (Some(expected), Some(actual)) => {
            if same_optional_i64(expected.material_index, actual.material_index) {
                "same-index".to_owned()
            } else if same_optional_u64(expected.material_slot, actual.material_slot) {
                "same-slot".to_owned()
            } else if same_optional_str(
                expected.material_name.as_deref(),
                actual.material_name.as_deref(),
            ) {
                "same-name".to_owned()
            } else {
                "different".to_owned()
            }
        }
        _ => "unknown".to_owned(),
    }
}

fn triangle_relation(expected: Option<&OwnerLabel>, actual: Option<&OwnerLabel>) -> String {
    match (expected.and_then(|label| label.triangle), actual.and_then(|label| label.triangle)) {
        (Some(left), Some(right)) if left == right => "same-triangle".to_owned(),
        (Some(left), Some(right)) if left.abs_diff(right) == 1 => "adjacent-triangle-index".to_owned(),
        (Some(_), Some(_)) => "different-triangle".to_owned(),
        _ => "unknown".to_owned(),
    }
}

fn projection_relation(expected: Option<&OwnerLabel>, actual: Option<&OwnerLabel>) -> String {
    let Some(expected) = expected else {
        return "unknown".to_owned();
    };
    let Some(actual) = actual else {
        return "unknown".to_owned();
    };
    let Some(expected_bounds) = expected.screen_bounds else {
        return "unknown".to_owned();
    };
    let Some(actual_bounds) = actual.screen_bounds else {
        return "unknown".to_owned();
    };
    if !screen_bounds_overlap(expected_bounds, actual_bounds, 1.0) {
        return "disjoint-screen-bounds".to_owned();
    }
    if same_webgl_depth(expected, actual, 0.001) {
        "overlap-depth-close".to_owned()
    } else {
        "overlap-depth-different".to_owned()
    }
}

fn same_optional_str(left: Option<&str>, right: Option<&str>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if !left.is_empty() && left == right)
}

fn same_optional_u64(left: Option<u64>, right: Option<u64>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left == right)
}

fn same_optional_i64(left: Option<i64>, right: Option<i64>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left == right)
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

fn screen_bounds_overlap(left: OwnerScreenBounds, right: OwnerScreenBounds, pad: f64) -> bool {
    left.min_x <= right.max_x + pad
        && left.max_x + pad >= right.min_x
        && left.min_y <= right.max_y + pad
        && left.max_y + pad >= right.min_y
}

fn same_webgl_depth(expected: &OwnerLabel, actual: &OwnerLabel, tolerance: f64) -> bool {
    match (expected.webgl_depth.or(expected.depth), actual.webgl_depth.or(actual.depth)) {
        (Some(left), Some(right)) => (left - right).abs() <= tolerance,
        _ => false,
    }
}

fn pass_label(label: Option<&OwnerLabel>) -> String {
    label
        .and_then(|label| label.pass.as_deref())
        .unwrap_or("unknown")
        .to_owned()
}

fn material_side_label(side: i64) -> String {
    match side {
        0 => "front".to_owned(),
        1 => "back".to_owned(),
        2 => "double".to_owned(),
        value => format!("side:{value}"),
    }
}

fn optional_bool_label(value: Option<bool>) -> String {
    match value {
        Some(true) => "true".to_owned(),
        Some(false) => "false".to_owned(),
        None => "unknown".to_owned(),
    }
}

fn bump_transition_pixels(
    map: &mut BTreeMap<(u32, u32), OwnerTransitionPixels>,
    expected: u32,
    actual: u32,
    pixel: OwnerPixel,
) {
    map.entry((expected, actual))
        .and_modify(|entry| entry.add(pixel))
        .or_insert_with(|| OwnerTransitionPixels::new(pixel));
}

impl OwnerTransitionPixels {
    fn new(pixel: OwnerPixel) -> Self {
        Self {
            bounds: OwnerPixelBounds {
                min_x: pixel.x,
                min_y: pixel.y,
                max_x: pixel.x,
                max_y: pixel.y,
            },
            sample_pixels: vec![pixel],
        }
    }

    fn add(&mut self, pixel: OwnerPixel) {
        self.bounds.min_x = self.bounds.min_x.min(pixel.x);
        self.bounds.min_y = self.bounds.min_y.min(pixel.y);
        self.bounds.max_x = self.bounds.max_x.max(pixel.x);
        self.bounds.max_y = self.bounds.max_y.max(pixel.y);
        if self.sample_pixels.len() < 8 {
            self.sample_pixels.push(pixel);
        }
    }
}

fn top_transition_details(
    map: &BTreeMap<(u32, u32), u64>,
    pixels: &BTreeMap<(u32, u32), OwnerTransitionPixels>,
    expected_metadata: &HashMap<u32, OwnerLabel>,
    actual_metadata: &HashMap<u32, OwnerLabel>,
    top: usize,
) -> Vec<OwnerTransitionDetail> {
    let mut entries = map.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .1
            .cmp(left.1)
            .then_with(|| left.0.0.cmp(&right.0.0))
            .then_with(|| left.0.1.cmp(&right.0.1))
    });
    entries
        .into_iter()
        .take(top)
        .map(|((expected, actual), count)| OwnerTransitionDetail {
            expected: expected_metadata
                .get(expected)
                .cloned()
                .unwrap_or_else(|| OwnerLabel::from_id(*expected)),
            actual: actual_metadata
                .get(actual)
                .cloned()
                .unwrap_or_else(|| OwnerLabel::from_id(*actual)),
            count: *count,
            bounds: pixels.get(&(*expected, *actual)).map(|pixels| pixels.bounds),
            sample_pixels: pixels
                .get(&(*expected, *actual))
                .map(|pixels| pixels.sample_pixels.clone())
                .unwrap_or_default(),
        })
        .collect()
}

impl OwnerLabel {
    fn from_id(id: u32) -> Self {
        Self {
            id,
            ..Self::default()
        }
    }
}

fn neighborhood_contains(
    ids: &[u32],
    width: usize,
    height: usize,
    index: usize,
    target: u32,
    radius: isize,
) -> bool {
    let x = (index % width) as isize;
    let y = (index / width) as isize;
    (-radius..=radius).any(|dy| {
        (-radius..=radius).any(|dx| {
            let nx = x + dx;
            let ny = y + dy;
            nx >= 0
                && ny >= 0
                && (nx as usize) < width
                && (ny as usize) < height
                && ids[(ny as usize) * width + nx as usize] == target
        })
    })
}

fn bump_transition(map: &mut BTreeMap<(u32, u32), u64>, expected: u32, actual: u32) {
    *map.entry((expected, actual)).or_default() += 1;
}

fn top_transitions(map: BTreeMap<(u32, u32), u64>, top: usize) -> Vec<OwnerTransition> {
    let mut entries = map.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.0.cmp(&right.0.0))
            .then_with(|| left.0.1.cmp(&right.0.1))
    });
    entries
        .into_iter()
        .take(top)
        .map(|((expected, actual), count)| OwnerTransition {
            expected,
            actual,
            count,
        })
        .collect()
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn read_imqraw_rgba8(path: &Path, index: usize) -> Result<RgbaImage, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let bundle = decode_imqraw_bundle(&bytes)?;
    let record = bundle.select_index(index)?;
    record_to_rgba8(path, index, record)
}

fn record_to_rgba8(
    path: &Path,
    index: usize,
    record: &RawImageRecord,
) -> Result<RgbaImage, Box<dyn Error>> {
    if record.frame.format().pixel_format != PixelFormat::Rgba8 {
        return Err(format!(
            "{}[{index}]: expected Rgba8, got {:?}",
            path.display(),
            record.frame.format().pixel_format
        )
        .into());
    }
    let dims = record.frame.dimensions();
    let width = usize::try_from(dims.width)?;
    let height = usize::try_from(dims.height)?;
    let row_bytes = width
        .checked_mul(4)
        .ok_or("RGBA row byte count overflows usize")?;
    let plane = record
        .frame
        .owned_planes()
        .first()
        .ok_or("Rgba8 imqraw record has no plane")?;
    if plane.stride < row_bytes {
        return Err(format!(
            "{}[{index}]: stride {} is smaller than row bytes {row_bytes}",
            path.display(),
            plane.stride
        )
        .into());
    }
    let required = height
        .checked_sub(1)
        .and_then(|last_row| last_row.checked_mul(plane.stride))
        .and_then(|last_row_start| last_row_start.checked_add(row_bytes))
        .ok_or("RGBA buffer size overflows usize")?;
    if plane.data.len() < required {
        return Err(format!(
            "{}[{index}]: plane has {} bytes, needs {required}",
            path.display(),
            plane.data.len()
        )
        .into());
    }
    let mut rgba = Vec::with_capacity(row_bytes * height);
    for row in 0..height {
        let offset = row * plane.stride;
        rgba.extend_from_slice(&plane.data[offset..offset + row_bytes]);
    }
    Ok(RgbaImage {
        width,
        height,
        rgba,
    })
}

fn read_owner_metadata_for_imqraw(path: &Path) -> Result<HashMap<u32, OwnerLabel>, Box<dyn Error>> {
    let rgba_json = path.with_extension("rgba.json");
    if !rgba_json.exists() {
        return Ok(HashMap::new());
    }
    let value = serde_json::from_slice::<Value>(&fs::read(&rgba_json)?)?;
    let owners = value
        .pointer("/renderer/diagnosticOwnerIds")
        .or_else(|| value.pointer("/reference/renderer/diagnosticOwnerIds"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(owners
        .iter()
        .filter_map(owner_label)
        .map(|label| (label.id, label))
        .collect())
}

fn owner_label(value: &Value) -> Option<OwnerLabel> {
    Some(OwnerLabel {
        id: u32::try_from(value.get("id")?.as_u64()?).ok()?,
        material_name: string_field(value, "materialName"),
        pass: string_field(value, "pass"),
        node_name: string_field(value, "nodeName"),
        mesh_name: string_field(value, "meshName"),
        node_index: value.get("nodeIndex").and_then(Value::as_u64),
        mesh_index: value.get("meshIndex").and_then(Value::as_u64),
        primitive_index: value.get("primitiveIndex").and_then(Value::as_u64),
        material_index: value.get("materialIndex").and_then(Value::as_i64),
        material_slot: value.get("materialSlot").and_then(Value::as_u64),
        triangle: value.get("triangle").and_then(Value::as_u64),
        render_order: value.get("renderOrder").and_then(Value::as_i64),
        draw_index: value.get("drawIndex").and_then(Value::as_u64),
        material_type: string_field(value, "materialType"),
        front_face: string_field(value, "frontFace"),
        cull_mode: string_field(value, "cullMode"),
        material_side: value.get("side").and_then(Value::as_i64),
        alpha_mode: string_field(value, "alphaMode"),
        alpha_test: value.get("alphaTest").and_then(Value::as_f64),
        opacity: value.get("opacity").and_then(Value::as_f64),
        transparent: value.get("transparent").and_then(Value::as_bool),
        depth_write: value.get("depthWrite").and_then(Value::as_bool),
        depth_test: value.get("depthTest").and_then(Value::as_bool),
        depth_compare: string_field(value, "depthCompare"),
        blend: value.get("blend").and_then(Value::as_bool),
        blending: value.get("blending").and_then(Value::as_i64),
        premultiplied_alpha: value.get("premultipliedAlpha").and_then(Value::as_bool),
        alpha_cutoff: value.get("alphaCutoff").and_then(Value::as_f64),
        depth_bias: value.get("depthBias").and_then(Value::as_f64),
        screen_bounds: owner_screen_bounds(value.get("screenBounds")),
        depth: value.get("depth").and_then(Value::as_f64),
        webgl_depth: value.get("webglDepth").and_then(Value::as_f64),
        depth_range: string_field(value, "depthRange"),
        screen_signed_area: value.get("screenSignedArea").and_then(Value::as_f64),
        front_facing: value.get("frontFacing").and_then(Value::as_bool),
        gpu_front_facing: value.get("gpuFrontFacing").and_then(Value::as_bool),
        visible_by_cull_policy: value.get("visibleByCullPolicy").and_then(Value::as_bool),
    })
}

fn owner_screen_bounds(value: Option<&Value>) -> Option<OwnerScreenBounds> {
    let value = value?;
    Some(OwnerScreenBounds {
        min_x: value.get("minX")?.as_f64()?,
        min_y: value.get("minY")?.as_f64()?,
        max_x: value.get("maxX")?.as_f64()?,
        max_y: value.get("maxY")?.as_f64()?,
    })
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(ToOwned::to_owned)
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn self_test() -> Result<(), Box<dyn Error>> {
    let expected = RgbaImage {
        width: 4,
        height: 1,
        rgba: vec![
            1, 0, 0, 255, 2, 0, 0, 255, 5, 0, 0, 255, 0, 0, 0, 255,
        ],
    };
    let actual = RgbaImage {
        width: 4,
        height: 1,
        rgba: vec![
            1, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255, 6, 0, 0, 255,
        ],
    };
    let expected_metadata = HashMap::from([
        (
            2,
            OwnerLabel {
                id: 2,
                pass: Some("outline".to_owned()),
                material_side: Some(1),
                cull_mode: Some("back".to_owned()),
                front_face: Some("cw".to_owned()),
                front_facing: Some(false),
                gpu_front_facing: Some(true),
                visible_by_cull_policy: Some(true),
                depth_write: Some(true),
                ..OwnerLabel::default()
            },
        ),
        (
            5,
            OwnerLabel {
                id: 5,
                pass: Some("base".to_owned()),
                material_side: Some(0),
                cull_mode: Some("back".to_owned()),
                front_face: Some("ccw".to_owned()),
                front_facing: Some(true),
                gpu_front_facing: Some(true),
                visible_by_cull_policy: Some(true),
                depth_write: Some(true),
                ..OwnerLabel::default()
            },
        ),
    ]);
    let actual_metadata = HashMap::from([
        (
            3,
            OwnerLabel {
                id: 3,
                pass: Some("base".to_owned()),
                cull_mode: Some("back".to_owned()),
                front_face: Some("ccw".to_owned()),
                front_facing: Some(false),
                gpu_front_facing: Some(true),
                visible_by_cull_policy: Some(true),
                depth_write: Some(true),
                ..OwnerLabel::default()
            },
        ),
        (
            4,
            OwnerLabel {
                id: 4,
                pass: Some("outline".to_owned()),
                cull_mode: Some("front".to_owned()),
                front_face: Some("cw".to_owned()),
                front_facing: Some(true),
                gpu_front_facing: Some(true),
                visible_by_cull_policy: Some(false),
                depth_write: Some(false),
                ..OwnerLabel::default()
            },
        ),
    ]);
    let report = compare_owner_images(
        "expected".to_owned(),
        "actual".to_owned(),
        &expected,
        &actual,
        &expected_metadata,
        &actual_metadata,
        8,
    )?;
    assert_eq!(report.expected_nonzero, 3);
    assert_eq!(report.actual_nonzero, 4);
    assert_eq!(report.shared_nonzero, 3);
    assert_eq!(report.exact_owner_matches, 1);
    assert_eq!(report.expected_only, 0);
    assert_eq!(report.actual_only, 1);
    assert_eq!(report.mismatched_shared_nonzero, 2);
    assert_eq!(
        report.actual_not_visible_by_cull_policy_shared_nonzero,
        1
    );
    assert_eq!(
        report.actual_not_visible_by_cull_policy_mismatched_shared_nonzero,
        1
    );
    assert_eq!(report.top_expected_to_actual[0].expected, 2);
    assert_eq!(report.top_expected_to_actual[0].actual, 3);
    assert!(report.top_actual_cull_visibility.iter().any(|transition| {
        transition.actual_pass == "outline"
            && transition.actual_cull_mode == "front"
            && transition.actual_front_face == "cw"
            && transition.actual_visible_by_cull_policy == "false"
            && transition.count == 1
    }));
    assert!(report.top_render_policy_transitions.iter().any(|transition| {
        transition.expected_side == "back"
            && transition.expected_cull_mode == "back"
            && transition.expected_front_face == "cw"
            && transition.expected_gpu_front_facing == "true"
            && transition.expected_visible_by_cull_policy == "true"
            && transition.actual_cull_mode == "back"
            && transition.actual_gpu_front_facing == "true"
            && transition.actual_visible_by_cull_policy == "true"
            && transition.count == 1
    }));
    Ok(())
}
