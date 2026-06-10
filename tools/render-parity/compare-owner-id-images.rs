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
    exact_owner_match_ratio: f64,
    expected_neighborhood_1px_ratio: f64,
    actual_neighborhood_1px_ratio: f64,
    max_expected_owner: u32,
    max_actual_owner: u32,
    top_owner_id_deltas: Vec<OwnerIdDelta>,
    top_pass_transitions: Vec<OwnerPassTransition>,
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
    triangle: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
struct OwnerTransitionDetail {
    expected: OwnerLabel,
    actual: OwnerLabel,
    count: u64,
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
    let mut expected_to_actual = BTreeMap::new();
    let mut actual_to_expected = BTreeMap::new();
    let mut owner_id_deltas = BTreeMap::new();
    let mut pass_transitions = BTreeMap::new();

    for (index, (&expected_id, &actual_id)) in expected_ids.iter().zip(&actual_ids).enumerate() {
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
                if left == right {
                    exact_owner_matches += 1;
                } else {
                    mismatched_shared_nonzero += 1;
                    bump_transition(&mut expected_to_actual, left, right);
                    bump_transition(&mut actual_to_expected, right, left);
                    bump_pass_transition(
                        &mut pass_transitions,
                        expected_metadata.get(&left),
                        actual_metadata.get(&right),
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
        top_expected_to_actual: top_transitions(expected_to_actual.clone(), top),
        top_actual_to_expected: top_transitions(actual_to_expected.clone(), top),
        top_expected_to_actual_details: top_transition_details(
            &expected_to_actual,
            expected_metadata,
            actual_metadata,
            top,
        ),
        top_actual_to_expected_details: top_transition_details(
            &actual_to_expected,
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

fn top_transition_details(
    map: &BTreeMap<(u32, u32), u64>,
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
        triangle: value.get("triangle").and_then(Value::as_u64),
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
        width: 3,
        height: 1,
        rgba: vec![1, 0, 0, 255, 2, 0, 0, 255, 0, 0, 0, 255],
    };
    let actual = RgbaImage {
        width: 3,
        height: 1,
        rgba: vec![1, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255],
    };
    let report = compare_owner_images(
        "expected".to_owned(),
        "actual".to_owned(),
        &expected,
        &actual,
        &HashMap::new(),
        &HashMap::new(),
        8,
    )?;
    assert_eq!(report.expected_nonzero, 2);
    assert_eq!(report.actual_nonzero, 3);
    assert_eq!(report.shared_nonzero, 2);
    assert_eq!(report.exact_owner_matches, 1);
    assert_eq!(report.expected_only, 0);
    assert_eq!(report.actual_only, 1);
    assert_eq!(report.mismatched_shared_nonzero, 1);
    assert_eq!(report.top_expected_to_actual[0].expected, 2);
    assert_eq!(report.top_expected_to_actual[0].actual, 3);
    Ok(())
}
