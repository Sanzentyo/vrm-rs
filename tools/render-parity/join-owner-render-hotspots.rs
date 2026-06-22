#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
vrm-adapter = { path = "../../crates/vrm-adapter" }
---

//! Join browser owner-id hotspot projections with Rust CPU render hotspot reports.

use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use vrm_adapter::{
    normalize_owner_diagnostic_material_name, rgb_distance_u8, RenderOwnerSampleKey,
    RenderOwnerSurfaceKey, RenderOwnerSurfaceRelation, RenderSamplePoint,
};

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
    browser_best_sample_rust_color_count: u64,
    browser_best_sample_actual_color_closer: u64,
    browser_best_sample_expected_color_closer: u64,
    browser_best_sample_color_tied: u64,
    browser_best_sample_mean_actual_rgb_distance: Option<f64>,
    browser_best_sample_mean_expected_rgb_distance: Option<f64>,
    browser_best_coverage_count: u64,
    browser_best_coverage_matches_rust_frontmost: u64,
    browser_best_coverage_matches_rust_expected_best_subpixel: u64,
    browser_best_coverage_matches_rust_actual_best_subpixel: u64,
    browser_best_coverage_sample_rust_color_count: u64,
    browser_best_coverage_sample_actual_color_closer: u64,
    browser_best_coverage_sample_expected_color_closer: u64,
    browser_best_coverage_sample_color_tied: u64,
    browser_best_coverage_sample_mean_actual_rgb_distance: Option<f64>,
    browser_best_coverage_sample_mean_expected_rgb_distance: Option<f64>,
    browser_best_to_rust_frontmost_relation: BTreeMap<String, u64>,
    browser_best_to_rust_expected_best_relation: BTreeMap<String, u64>,
    browser_best_to_rust_actual_best_relation: BTreeMap<String, u64>,
    browser_best_coverage_to_rust_frontmost_relation: BTreeMap<String, u64>,
    browser_best_coverage_to_rust_expected_best_relation: BTreeMap<String, u64>,
    browser_best_coverage_to_rust_actual_best_relation: BTreeMap<String, u64>,
    browser_best_expected_closer_to_expected_relation: BTreeMap<String, u64>,
    browser_best_actual_closer_to_actual_relation: BTreeMap<String, u64>,
    browser_best_coverage_expected_closer_to_expected_relation: BTreeMap<String, u64>,
    browser_best_coverage_actual_closer_to_actual_relation: BTreeMap<String, u64>,
    rendered_to_rust_frontmost: BTreeMap<String, u64>,
    rendered_to_rust_expected_best_subpixel: BTreeMap<String, u64>,
    browser_best_to_rust_expected_best_subpixel: BTreeMap<String, u64>,
    browser_best_coverage_to_rust_expected_best_subpixel: BTreeMap<String, u64>,
    pbr_best_to_frontmost_term_output: PbrTermOutputSummary,
    pbr_best_to_expected_term_output: PbrTermOutputSummary,
    pbr_coverage_to_frontmost_term_output: PbrTermOutputSummary,
    pbr_coverage_to_expected_term_output: PbrTermOutputSummary,
    top_disagreements: Vec<JoinedHotspotLine>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct PbrTermOutputSummary {
    count: u64,
    diffuse_lobe_sample_count: u64,
    mean_diffuse_lobe_rgb_distance: Option<f64>,
    specular_lobe_sample_count: u64,
    mean_specular_lobe_rgb_distance: Option<f64>,
    mean_direct_rgb_distance: Option<f64>,
    mean_ambient_rgb_distance: Option<f64>,
    mean_total_rgb_distance: Option<f64>,
    shading_normal_three_js_sample_count: u64,
    mean_shading_normal_three_js_distance: Option<f64>,
    shading_normal_wgpu_compat_sample_count: u64,
    mean_shading_normal_wgpu_compat_distance: Option<f64>,
    shading_normal_three_js_rust_space_sample_count: u64,
    mean_shading_normal_three_js_rust_space_distance: Option<f64>,
    normal_source_pairs: BTreeMap<String, u64>,
    mean_output_rgb_distance: Option<f64>,
}

#[derive(Clone, Debug, Default)]
struct PbrTermOutputAccumulator {
    count: u64,
    diffuse_lobe_distance: MeanAccumulator,
    specular_lobe_distance: MeanAccumulator,
    direct_distance_sum: f64,
    ambient_distance_sum: f64,
    total_distance_sum: f64,
    shading_normal_three_js_distance: MeanAccumulator,
    shading_normal_wgpu_compat_distance: MeanAccumulator,
    shading_normal_three_js_rust_space_distance: MeanAccumulator,
    normal_source_pairs: BTreeMap<String, u64>,
    output_distance_sum: f64,
}

#[derive(Clone, Debug, Default)]
struct MeanAccumulator {
    count: u64,
    sum: f64,
}

#[derive(Clone, Debug)]
struct PbrTermSample {
    normal_source: Option<String>,
    diffuse_lobe_rgb: Option<[f64; 3]>,
    specular_lobe_rgb: Option<[f64; 3]>,
    direct_rgb: [f64; 3],
    ambient_rgb: [f64; 3],
    direct_plus_ambient_rgb: [f64; 3],
    shading_normal_three_js: Option<[f64; 3]>,
    shading_normal_wgpu_compat: Option<[f64; 3]>,
    shading_normal: Option<[f64; 3]>,
}

#[derive(Clone, Debug, Serialize)]
struct JoinedHotspotLine {
    x: u64,
    y: u64,
    rendered_owner: Option<SurfaceSummary>,
    browser_best_subpixel: Option<SurfaceSummary>,
    browser_best_subpixel_sample: Option<[f64; 2]>,
    browser_best_coverage: Option<SurfaceSummary>,
    browser_best_coverage_sample: Option<[f64; 2]>,
    browser_best_coverage_area_pixels: Option<f64>,
    browser_best_coverage_point_count: Option<u64>,
    browser_rendered_depth_rank: Option<u64>,
    rust_frontmost: Option<SurfaceSummary>,
    rust_expected_best_subpixel: Option<SurfaceSummary>,
    rust_expected_best_subpixel_sample: Option<[f64; 2]>,
    rust_actual_best_subpixel: Option<SurfaceSummary>,
    rust_actual_best_subpixel_sample: Option<[f64; 2]>,
    browser_best_sample_cpu_base_color: Option<[u64; 4]>,
    actual_rgba: Option<[u64; 4]>,
    expected_rgba: Option<[u64; 4]>,
    actual_expected_rgb_distance: Option<f64>,
    browser_best_sample_actual_rgb_distance: Option<f64>,
    browser_best_sample_expected_rgb_distance: Option<f64>,
    browser_best_coverage_sample_cpu_base_color: Option<[u64; 4]>,
    browser_best_coverage_sample_actual_rgb_distance: Option<f64>,
    browser_best_coverage_sample_expected_rgb_distance: Option<f64>,
    browser_best_pbr_terms: String,
    browser_best_coverage_pbr_terms: String,
    rust_frontmost_pbr_terms: String,
    rust_expected_best_pbr_terms: String,
    browser_best_to_expected_relation: String,
    browser_best_coverage_to_expected_relation: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
struct SurfaceSummary {
    material_name: String,
    triangle: u64,
}

impl SurfaceSummary {
    fn owner_key(&self) -> RenderOwnerSurfaceKey {
        RenderOwnerSurfaceKey::new(self.material_name.clone(), self.triangle)
    }
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
    let owner_path = options
        .owner_hotspots
        .as_ref()
        .ok_or("missing --owner-hotspots")?;
    let rust_path = options
        .rust_hotspots
        .as_ref()
        .ok_or("missing --rust-hotspots")?;
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
        browser_best_sample_rust_color_count: 0,
        browser_best_sample_actual_color_closer: 0,
        browser_best_sample_expected_color_closer: 0,
        browser_best_sample_color_tied: 0,
        browser_best_sample_mean_actual_rgb_distance: None,
        browser_best_sample_mean_expected_rgb_distance: None,
        browser_best_coverage_count: 0,
        browser_best_coverage_matches_rust_frontmost: 0,
        browser_best_coverage_matches_rust_expected_best_subpixel: 0,
        browser_best_coverage_matches_rust_actual_best_subpixel: 0,
        browser_best_coverage_sample_rust_color_count: 0,
        browser_best_coverage_sample_actual_color_closer: 0,
        browser_best_coverage_sample_expected_color_closer: 0,
        browser_best_coverage_sample_color_tied: 0,
        browser_best_coverage_sample_mean_actual_rgb_distance: None,
        browser_best_coverage_sample_mean_expected_rgb_distance: None,
        browser_best_to_rust_frontmost_relation: BTreeMap::new(),
        browser_best_to_rust_expected_best_relation: BTreeMap::new(),
        browser_best_to_rust_actual_best_relation: BTreeMap::new(),
        browser_best_coverage_to_rust_frontmost_relation: BTreeMap::new(),
        browser_best_coverage_to_rust_expected_best_relation: BTreeMap::new(),
        browser_best_coverage_to_rust_actual_best_relation: BTreeMap::new(),
        browser_best_expected_closer_to_expected_relation: BTreeMap::new(),
        browser_best_actual_closer_to_actual_relation: BTreeMap::new(),
        browser_best_coverage_expected_closer_to_expected_relation: BTreeMap::new(),
        browser_best_coverage_actual_closer_to_actual_relation: BTreeMap::new(),
        rendered_to_rust_frontmost: BTreeMap::new(),
        rendered_to_rust_expected_best_subpixel: BTreeMap::new(),
        browser_best_to_rust_expected_best_subpixel: BTreeMap::new(),
        browser_best_coverage_to_rust_expected_best_subpixel: BTreeMap::new(),
        pbr_best_to_frontmost_term_output: PbrTermOutputSummary::default(),
        pbr_best_to_expected_term_output: PbrTermOutputSummary::default(),
        pbr_coverage_to_frontmost_term_output: PbrTermOutputSummary::default(),
        pbr_coverage_to_expected_term_output: PbrTermOutputSummary::default(),
        top_disagreements: Vec::new(),
    };
    let mut browser_best_actual_distance_sum = 0.0;
    let mut browser_best_expected_distance_sum = 0.0;
    let mut browser_best_coverage_actual_distance_sum = 0.0;
    let mut browser_best_coverage_expected_distance_sum = 0.0;
    let mut pbr_best_frontmost = PbrTermOutputAccumulator::default();
    let mut pbr_best_expected = PbrTermOutputAccumulator::default();
    let mut pbr_coverage_frontmost = PbrTermOutputAccumulator::default();
    let mut pbr_coverage_expected = PbrTermOutputAccumulator::default();

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
        let browser_best = surface_at(
            owner_hotspot,
            "/renderedOwnerRecovery/bestSubpixel/candidate",
        );
        let browser_best_coverage = surface_at(
            owner_hotspot,
            "/renderedOwnerRecovery/bestCoverage/candidate",
        );
        let rust_frontmost = surface_at(rust_hotspot, "/frontmost_visible");
        let rust_expected = surface_at(rust_hotspot, "/best_subpixel_visible_expected/candidate");
        let rust_actual = surface_at(rust_hotspot, "/best_subpixel_visible_actual/candidate");
        let browser_best_sample =
            number_pair(owner_hotspot.pointer("/renderedOwnerRecovery/bestSubpixel/sampleCenter"));
        let browser_best_coverage_sample =
            number_pair(owner_hotspot.pointer("/renderedOwnerRecovery/bestCoverage/sampleCenter"));
        let actual_rgba = rgba_field(rust_hotspot, "actual");
        let expected_rgba = rgba_field(rust_hotspot, "expected");
        let actual_expected_rgb_distance = actual_rgba
            .zip(expected_rgba)
            .map(|(actual, expected)| rgb_distance_u64(actual, expected));
        let browser_best_coverage_area = f64_at(
            owner_hotspot,
            "/renderedOwnerRecovery/bestCoverage/coverageAreaPixels",
        );
        let browser_best_coverage_point_count = u64_at(
            owner_hotspot,
            "/renderedOwnerRecovery/bestCoverage/coveragePointCount",
        );
        let browser_best_sample_color = browser_best
            .as_ref()
            .zip(browser_best_sample)
            .map(|(surface, sample)| RenderOwnerSampleKey::from_pair(surface.owner_key(), sample))
            .and_then(|sample_key| rust_color_for_owner_sample(rust_hotspot, &sample_key));
        let browser_best_coverage_sample_color = browser_best_coverage
            .as_ref()
            .zip(browser_best_coverage_sample)
            .map(|(surface, sample)| RenderOwnerSampleKey::from_pair(surface.owner_key(), sample))
            .and_then(|sample_key| rust_color_for_owner_sample(rust_hotspot, &sample_key));
        let browser_best_sample_actual_distance = browser_best_sample_color
            .zip(actual_rgba)
            .map(|(color, actual)| rgb_distance_u64(color, actual));
        let browser_best_sample_expected_distance = browser_best_sample_color
            .zip(expected_rgba)
            .map(|(color, expected)| rgb_distance_u64(color, expected));
        let browser_best_coverage_sample_actual_distance = browser_best_coverage_sample_color
            .zip(actual_rgba)
            .map(|(color, actual)| rgb_distance_u64(color, actual));
        let browser_best_coverage_sample_expected_distance = browser_best_coverage_sample_color
            .zip(expected_rgba)
            .map(|(color, expected)| rgb_distance_u64(color, expected));

        if let (Some(actual), Some(expected)) = (
            browser_best_sample_actual_distance,
            browser_best_sample_expected_distance,
        ) {
            report.browser_best_sample_rust_color_count += 1;
            browser_best_actual_distance_sum += actual;
            browser_best_expected_distance_sum += expected;
            match actual
                .partial_cmp(&expected)
                .unwrap_or(std::cmp::Ordering::Equal)
            {
                std::cmp::Ordering::Less => report.browser_best_sample_actual_color_closer += 1,
                std::cmp::Ordering::Greater => {
                    report.browser_best_sample_expected_color_closer += 1
                }
                std::cmp::Ordering::Equal => report.browser_best_sample_color_tied += 1,
            }
            match actual
                .partial_cmp(&expected)
                .unwrap_or(std::cmp::Ordering::Equal)
            {
                std::cmp::Ordering::Less => bump_relation(
                    &mut report.browser_best_actual_closer_to_actual_relation,
                    browser_best.as_ref(),
                    rust_actual.as_ref(),
                ),
                std::cmp::Ordering::Greater => bump_relation(
                    &mut report.browser_best_expected_closer_to_expected_relation,
                    browser_best.as_ref(),
                    rust_expected.as_ref(),
                ),
                std::cmp::Ordering::Equal => {}
            }
        }

        if let (Some(actual), Some(expected)) = (
            browser_best_coverage_sample_actual_distance,
            browser_best_coverage_sample_expected_distance,
        ) {
            report.browser_best_coverage_sample_rust_color_count += 1;
            browser_best_coverage_actual_distance_sum += actual;
            browser_best_coverage_expected_distance_sum += expected;
            match actual
                .partial_cmp(&expected)
                .unwrap_or(std::cmp::Ordering::Equal)
            {
                std::cmp::Ordering::Less => {
                    report.browser_best_coverage_sample_actual_color_closer += 1
                }
                std::cmp::Ordering::Greater => {
                    report.browser_best_coverage_sample_expected_color_closer += 1
                }
                std::cmp::Ordering::Equal => report.browser_best_coverage_sample_color_tied += 1,
            }
            match actual
                .partial_cmp(&expected)
                .unwrap_or(std::cmp::Ordering::Equal)
            {
                std::cmp::Ordering::Less => bump_relation(
                    &mut report.browser_best_coverage_actual_closer_to_actual_relation,
                    browser_best_coverage.as_ref(),
                    rust_actual.as_ref(),
                ),
                std::cmp::Ordering::Greater => bump_relation(
                    &mut report.browser_best_coverage_expected_closer_to_expected_relation,
                    browser_best_coverage.as_ref(),
                    rust_expected.as_ref(),
                ),
                std::cmp::Ordering::Equal => {}
            }
        }

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
        if browser_best_coverage.is_some() {
            report.browser_best_coverage_count += 1;
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
        add_match_counts(
            &mut report.browser_best_coverage_matches_rust_frontmost,
            browser_best_coverage.as_ref(),
            rust_frontmost.as_ref(),
        );
        add_match_counts(
            &mut report.browser_best_coverage_matches_rust_expected_best_subpixel,
            browser_best_coverage.as_ref(),
            rust_expected.as_ref(),
        );
        add_match_counts(
            &mut report.browser_best_coverage_matches_rust_actual_best_subpixel,
            browser_best_coverage.as_ref(),
            rust_actual.as_ref(),
        );
        bump_pair(
            &mut report.rendered_to_rust_frontmost,
            rendered.as_ref(),
            rust_frontmost.as_ref(),
        );
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
        bump_pair(
            &mut report.browser_best_coverage_to_rust_expected_best_subpixel,
            browser_best_coverage.as_ref(),
            rust_expected.as_ref(),
        );
        bump_relation(
            &mut report.browser_best_to_rust_frontmost_relation,
            browser_best.as_ref(),
            rust_frontmost.as_ref(),
        );
        bump_relation(
            &mut report.browser_best_to_rust_expected_best_relation,
            browser_best.as_ref(),
            rust_expected.as_ref(),
        );
        bump_relation(
            &mut report.browser_best_to_rust_actual_best_relation,
            browser_best.as_ref(),
            rust_actual.as_ref(),
        );
        bump_relation(
            &mut report.browser_best_coverage_to_rust_frontmost_relation,
            browser_best_coverage.as_ref(),
            rust_frontmost.as_ref(),
        );
        bump_relation(
            &mut report.browser_best_coverage_to_rust_expected_best_relation,
            browser_best_coverage.as_ref(),
            rust_expected.as_ref(),
        );
        bump_relation(
            &mut report.browser_best_coverage_to_rust_actual_best_relation,
            browser_best_coverage.as_ref(),
            rust_actual.as_ref(),
        );
        if same_surface(browser_best.as_ref(), rust_frontmost.as_ref()) {
            pbr_best_frontmost.observe(
                browser_pbr_terms(
                    owner_hotspot,
                    "/renderedOwnerRecovery/bestSubpixel/candidate/browserPbrTerms",
                ),
                rust_pbr_terms(rust_hotspot, "/frontmost_visible/pbr_terms"),
                actual_expected_rgb_distance,
            );
        }
        if same_surface(browser_best.as_ref(), rust_expected.as_ref()) {
            pbr_best_expected.observe(
                browser_pbr_terms(
                    owner_hotspot,
                    "/renderedOwnerRecovery/bestSubpixel/candidate/browserPbrTerms",
                ),
                rust_pbr_terms(
                    rust_hotspot,
                    "/best_subpixel_visible_expected/candidate/pbr_terms",
                ),
                actual_expected_rgb_distance,
            );
        }
        if same_surface(browser_best_coverage.as_ref(), rust_frontmost.as_ref()) {
            pbr_coverage_frontmost.observe(
                browser_pbr_terms(
                    owner_hotspot,
                    "/renderedOwnerRecovery/bestCoverage/candidate/browserPbrTerms",
                ),
                rust_pbr_terms(rust_hotspot, "/frontmost_visible/pbr_terms"),
                actual_expected_rgb_distance,
            );
        }
        if same_surface(browser_best_coverage.as_ref(), rust_expected.as_ref()) {
            pbr_coverage_expected.observe(
                browser_pbr_terms(
                    owner_hotspot,
                    "/renderedOwnerRecovery/bestCoverage/candidate/browserPbrTerms",
                ),
                rust_pbr_terms(
                    rust_hotspot,
                    "/best_subpixel_visible_expected/candidate/pbr_terms",
                ),
                actual_expected_rgb_distance,
            );
        }

        if report.top_disagreements.len() < top && rendered.as_ref() != rust_expected.as_ref() {
            let browser_best_to_expected_relation =
                relation_label(browser_best.as_ref(), rust_expected.as_ref()).to_owned();
            let browser_best_coverage_to_expected_relation =
                relation_label(browser_best_coverage.as_ref(), rust_expected.as_ref()).to_owned();
            report.top_disagreements.push(JoinedHotspotLine {
                x,
                y,
                rendered_owner: rendered,
                browser_best_subpixel: browser_best,
                browser_best_subpixel_sample: browser_best_sample,
                browser_best_coverage,
                browser_best_coverage_sample,
                browser_best_coverage_area_pixels: browser_best_coverage_area,
                browser_best_coverage_point_count,
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
                browser_best_sample_cpu_base_color: browser_best_sample_color,
                actual_rgba,
                expected_rgba,
                actual_expected_rgb_distance,
                browser_best_sample_actual_rgb_distance: browser_best_sample_actual_distance,
                browser_best_sample_expected_rgb_distance: browser_best_sample_expected_distance,
                browser_best_coverage_sample_cpu_base_color: browser_best_coverage_sample_color,
                browser_best_coverage_sample_actual_rgb_distance:
                    browser_best_coverage_sample_actual_distance,
                browser_best_coverage_sample_expected_rgb_distance:
                    browser_best_coverage_sample_expected_distance,
                browser_best_pbr_terms: browser_pbr_terms_summary(
                    owner_hotspot,
                    "/renderedOwnerRecovery/bestSubpixel/candidate/browserPbrTerms",
                ),
                browser_best_coverage_pbr_terms: browser_pbr_terms_summary(
                    owner_hotspot,
                    "/renderedOwnerRecovery/bestCoverage/candidate/browserPbrTerms",
                ),
                rust_frontmost_pbr_terms: rust_pbr_terms_summary(
                    rust_hotspot,
                    "/frontmost_visible/pbr_terms",
                ),
                rust_expected_best_pbr_terms: rust_pbr_terms_summary(
                    rust_hotspot,
                    "/best_subpixel_visible_expected/candidate/pbr_terms",
                ),
                browser_best_to_expected_relation,
                browser_best_coverage_to_expected_relation,
            });
        }
    }

    if report.browser_best_sample_rust_color_count > 0 {
        let count = report.browser_best_sample_rust_color_count as f64;
        report.browser_best_sample_mean_actual_rgb_distance =
            Some(browser_best_actual_distance_sum / count);
        report.browser_best_sample_mean_expected_rgb_distance =
            Some(browser_best_expected_distance_sum / count);
    }
    if report.browser_best_coverage_sample_rust_color_count > 0 {
        let count = report.browser_best_coverage_sample_rust_color_count as f64;
        report.browser_best_coverage_sample_mean_actual_rgb_distance =
            Some(browser_best_coverage_actual_distance_sum / count);
        report.browser_best_coverage_sample_mean_expected_rgb_distance =
            Some(browser_best_coverage_expected_distance_sum / count);
    }
    report.pbr_best_to_frontmost_term_output = pbr_best_frontmost.into_summary();
    report.pbr_best_to_expected_term_output = pbr_best_expected.into_summary();
    report.pbr_coverage_to_frontmost_term_output = pbr_coverage_frontmost.into_summary();
    report.pbr_coverage_to_expected_term_output = pbr_coverage_expected.into_summary();

    Ok(report)
}

fn add_match_counts(
    count: &mut u64,
    left: Option<&SurfaceSummary>,
    right: Option<&SurfaceSummary>,
) {
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

fn bump_relation(
    counts: &mut BTreeMap<String, u64>,
    left: Option<&SurfaceSummary>,
    right: Option<&SurfaceSummary>,
) {
    *counts
        .entry(relation_label(left, right).to_owned())
        .or_default() += 1;
}

fn relation_label(left: Option<&SurfaceSummary>, right: Option<&SurfaceSummary>) -> &'static str {
    let left = left.map(SurfaceSummary::owner_key);
    let right = right.map(SurfaceSummary::owner_key);
    left.as_ref()
        .map(|left| left.relation_to(right.as_ref()).as_str())
        .unwrap_or(RenderOwnerSurfaceRelation::Missing.as_str())
}

fn same_surface(left: Option<&SurfaceSummary>, right: Option<&SurfaceSummary>) -> bool {
    left.zip(right).is_some_and(|(left, right)| left == right)
}

impl PbrTermOutputAccumulator {
    fn observe(
        &mut self,
        browser: Option<PbrTermSample>,
        rust: Option<PbrTermSample>,
        output_distance: Option<f64>,
    ) {
        let (Some(browser), Some(rust), Some(output_distance)) = (browser, rust, output_distance)
        else {
            return;
        };
        self.count += 1;
        self.diffuse_lobe_distance
            .observe_pair(browser.diffuse_lobe_rgb, rust.diffuse_lobe_rgb);
        self.specular_lobe_distance
            .observe_pair(browser.specular_lobe_rgb, rust.specular_lobe_rgb);
        self.direct_distance_sum += vec3_distance(browser.direct_rgb, rust.direct_rgb);
        self.ambient_distance_sum += vec3_distance(browser.ambient_rgb, rust.ambient_rgb);
        self.total_distance_sum += vec3_distance(
            browser.direct_plus_ambient_rgb,
            rust.direct_plus_ambient_rgb,
        );
        self.shading_normal_three_js_distance
            .observe_pair(browser.shading_normal_three_js, rust.shading_normal);
        self.shading_normal_wgpu_compat_distance
            .observe_pair(browser.shading_normal_wgpu_compat, rust.shading_normal);
        self.shading_normal_three_js_rust_space_distance
            .observe_pair(
                browser
                    .shading_normal_three_js
                    .map(browser_normal_to_rust_space),
                rust.shading_normal,
            );
        let browser_source = browser.normal_source.as_deref().unwrap_or("n/a");
        let rust_source = rust.normal_source.as_deref().unwrap_or("n/a");
        *self
            .normal_source_pairs
            .entry(format!("{browser_source} -> {rust_source}"))
            .or_insert(0) += 1;
        self.output_distance_sum += output_distance;
    }

    fn into_summary(self) -> PbrTermOutputSummary {
        let Some(count) = (self.count > 0).then_some(self.count as f64) else {
            return PbrTermOutputSummary::default();
        };
        PbrTermOutputSummary {
            count: self.count,
            diffuse_lobe_sample_count: self.diffuse_lobe_distance.count,
            mean_diffuse_lobe_rgb_distance: self.diffuse_lobe_distance.mean(),
            specular_lobe_sample_count: self.specular_lobe_distance.count,
            mean_specular_lobe_rgb_distance: self.specular_lobe_distance.mean(),
            mean_direct_rgb_distance: Some(self.direct_distance_sum / count),
            mean_ambient_rgb_distance: Some(self.ambient_distance_sum / count),
            mean_total_rgb_distance: Some(self.total_distance_sum / count),
            shading_normal_three_js_sample_count: self.shading_normal_three_js_distance.count,
            mean_shading_normal_three_js_distance: self.shading_normal_three_js_distance.mean(),
            shading_normal_wgpu_compat_sample_count: self.shading_normal_wgpu_compat_distance.count,
            mean_shading_normal_wgpu_compat_distance: self
                .shading_normal_wgpu_compat_distance
                .mean(),
            shading_normal_three_js_rust_space_sample_count: self
                .shading_normal_three_js_rust_space_distance
                .count,
            mean_shading_normal_three_js_rust_space_distance: self
                .shading_normal_three_js_rust_space_distance
                .mean(),
            normal_source_pairs: self.normal_source_pairs,
            mean_output_rgb_distance: Some(self.output_distance_sum / count),
        }
    }
}

fn browser_normal_to_rust_space(normal: [f64; 3]) -> [f64; 3] {
    [-normal[0], normal[1], -normal[2]]
}

impl MeanAccumulator {
    fn observe_pair(&mut self, left: Option<[f64; 3]>, right: Option<[f64; 3]>) {
        if let (Some(left), Some(right)) = (left, right) {
            self.count += 1;
            self.sum += vec3_distance(left, right);
        }
    }

    fn mean(self) -> Option<f64> {
        (self.count > 0).then_some(self.sum / self.count as f64)
    }
}

fn browser_pbr_terms(value: &Value, pointer: &str) -> Option<PbrTermSample> {
    let terms = value.pointer(pointer)?;
    Some(PbrTermSample {
        normal_source: value_string_path(terms, "normalSource"),
        diffuse_lobe_rgb: value_vec3_path(terms, "diffuseLobeRgb"),
        specular_lobe_rgb: value_vec3_path(terms, "specularLobeRgb"),
        direct_rgb: value_vec3_path(terms, "directRgb")?,
        ambient_rgb: value_vec3_path(terms, "ambientRgb")?,
        direct_plus_ambient_rgb: value_vec3_path(terms, "directPlusAmbientRgb")?,
        shading_normal_three_js: value_vec3_path(terms, "shadingNormalThreeJs"),
        shading_normal_wgpu_compat: value_vec3_path(terms, "shadingNormalWgpuCompat"),
        shading_normal: None,
    })
}

fn rust_pbr_terms(value: &Value, pointer: &str) -> Option<PbrTermSample> {
    let terms = value.pointer(pointer)?;
    Some(PbrTermSample {
        normal_source: value_string_path(terms, "normal_source"),
        diffuse_lobe_rgb: value_vec3_path(terms, "diffuse_lobe_rgb"),
        specular_lobe_rgb: value_vec3_path(terms, "specular_lobe_rgb"),
        direct_rgb: value_vec3_path(terms, "direct_rgb")?,
        ambient_rgb: value_vec3_path(terms, "ambient_rgb")?,
        direct_plus_ambient_rgb: value_vec3_path(terms, "direct_plus_ambient_rgb")?,
        shading_normal_three_js: None,
        shading_normal_wgpu_compat: None,
        shading_normal: value_vec3_path(terms, "shading_normal"),
    })
}

fn vec3_distance(left: [f64; 3], right: [f64; 3]) -> f64 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| {
            let delta = left - right;
            delta * delta
        })
        .sum::<f64>()
        .sqrt()
}

fn owner_surface(hotspot: &Value) -> Option<SurfaceSummary> {
    surface_at(hotspot, "/renderedOwner/owner")
        .or_else(|| surface_at(hotspot, "/renderedOwnerCandidate"))
}

fn surface_at(value: &Value, pointer: &str) -> Option<SurfaceSummary> {
    let value = value.pointer(pointer)?;
    Some(SurfaceSummary {
        material_name: normalize_owner_diagnostic_material_name(
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

fn rust_color_for_owner_sample(
    rust_hotspot: &Value,
    sample_key: &RenderOwnerSampleKey,
) -> Option<[u64; 4]> {
    ["coverage_visible_candidates", "subpixel_visible_candidates"]
        .iter()
        .find_map(|array_name| {
            rust_color_for_owner_sample_in_array(rust_hotspot, array_name, sample_key)
        })
}

fn rust_color_for_owner_sample_in_array(
    rust_hotspot: &Value,
    array_name: &str,
    sample_key: &RenderOwnerSampleKey,
) -> Option<[u64; 4]> {
    rust_hotspot
        .get(array_name)
        .and_then(Value::as_array)?
        .iter()
        .find_map(|candidate| {
            let candidate_surface = surface_at(candidate, "/candidate")?;
            let candidate_sample = number_pair(candidate.get("sample"))?;
            sample_key
                .matches(
                    &candidate_surface.owner_key(),
                    RenderSamplePoint::from_pair(candidate_sample),
                )
                .then(|| candidate.pointer("/candidate/cpu_base_color_rgba"))
                .flatten()
                .and_then(rgba_array)
        })
}

fn rgba_field(value: &Value, key: &str) -> Option<[u64; 4]> {
    value.get(key).and_then(rgba_array)
}

fn rgba_array(value: &Value) -> Option<[u64; 4]> {
    let values = value.as_array()?;
    Some([
        values.first()?.as_u64()?,
        values.get(1)?.as_u64()?,
        values.get(2)?.as_u64()?,
        values.get(3)?.as_u64()?,
    ])
}

fn rgb_distance_u64(left: [u64; 4], right: [u64; 4]) -> f64 {
    match (rgba_u64_to_u8(left), rgba_u64_to_u8(right)) {
        (Some(left), Some(right)) => rgb_distance_u8(left, right),
        _ => left
            .iter()
            .zip(right.iter())
            .take(3)
            .map(|(left, right)| {
                let delta = *left as f64 - *right as f64;
                delta * delta
            })
            .sum::<f64>()
            .sqrt(),
    }
}

fn rgba_u64_to_u8(rgba: [u64; 4]) -> Option<[u8; 4]> {
    Some([
        u8::try_from(rgba[0]).ok()?,
        u8::try_from(rgba[1]).ok()?,
        u8::try_from(rgba[2]).ok()?,
        u8::try_from(rgba[3]).ok()?,
    ])
}

fn number_pair(value: Option<&Value>) -> Option<[f64; 2]> {
    let values = value?.as_array()?;
    Some([values.first()?.as_f64()?, values.get(1)?.as_f64()?])
}

fn f64_at(value: &Value, pointer: &str) -> Option<f64> {
    value.pointer(pointer).and_then(Value::as_f64)
}

fn u64_at(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer).and_then(Value::as_u64)
}

fn browser_pbr_terms_summary(value: &Value, pointer: &str) -> String {
    let Some(terms) = value.pointer(pointer) else {
        return "n/a".to_owned();
    };
    if terms.is_null() {
        return "n/a".to_owned();
    }
    format!(
        "model={} nL/nV={}/{} normal={} uv={} tex={} base-uv={} base-tex={} geom={} shade3={} shade-wgpu={} tan3={} tan-wgpu={} diff={} spec={} direct={} amb={} total={} m/r={}/{} f90={} rough-used={}",
        string_path(terms, "model"),
        fmt_opt_f64(value_f64_path(terms, "nDotL")),
        fmt_opt_f64(value_f64_path(terms, "nDotV")),
        string_path(terms, "normalSource"),
        fmt_vec2(value_vec2_path(terms, "normalUv")),
        fmt_rgba(value_rgba_path(terms, "normalTextureRgba")),
        fmt_vec2(value_vec2_path(terms, "baseUv")),
        fmt_rgba(value_rgba_path(terms, "baseTextureRgba")),
        fmt_vec3(value_vec3_path(terms, "geometricNormal")),
        fmt_vec3(value_vec3_path(terms, "shadingNormalThreeJs")),
        fmt_vec3(value_vec3_path(terms, "shadingNormalWgpuCompat")),
        fmt_vec3(value_vec3_path(terms, "tangentSpaceNormalThreeJs")),
        fmt_vec3(value_vec3_path(terms, "tangentSpaceNormalWgpuCompat")),
        fmt_vec3(value_vec3_path(terms, "diffuseLobeRgb")),
        fmt_vec3(value_vec3_path(terms, "specularLobeRgb")),
        fmt_vec3(value_vec3_path(terms, "directRgb")),
        fmt_vec3(value_vec3_path(terms, "ambientRgb")),
        fmt_vec3(value_vec3_path(terms, "directPlusAmbientRgb")),
        fmt_opt_f64(value_f64_path(terms, "metalness")),
        fmt_opt_f64(value_f64_path(terms, "roughness")),
        fmt_opt_f64(value_f64_path(terms, "specularF90")),
        fmt_opt_f64(value_f64_path(terms, "roughnessUsed")),
    )
}

fn rust_pbr_terms_summary(value: &Value, pointer: &str) -> String {
    let Some(terms) = value.pointer(pointer) else {
        return "n/a".to_owned();
    };
    if terms.is_null() {
        return "n/a".to_owned();
    }
    format!(
        "nL/nV={}/{} normal={} uv={} tex={} geom={} shade={} tan3={} tan-wgpu={} diff={} spec={} direct={} amb={} total={}",
        fmt_opt_f64(value_f64_path(terms, "n_dot_l")),
        fmt_opt_f64(value_f64_path(terms, "n_dot_v")),
        string_path(terms, "normal_source"),
        fmt_vec2(value_vec2_path(terms, "normal_uv")),
        fmt_rgba(value_rgba_path(terms, "normal_texture_rgba")),
        fmt_vec3(value_vec3_path(terms, "geometric_normal")),
        fmt_vec3(value_vec3_path(terms, "shading_normal")),
        fmt_vec3(value_vec3_path(terms, "tangent_space_normal_three_js")),
        fmt_vec3(value_vec3_path(
            terms,
            "tangent_space_normal_wgpu_compat",
        )),
        fmt_vec3(value_vec3_path(terms, "diffuse_lobe_rgb")),
        fmt_vec3(value_vec3_path(terms, "specular_lobe_rgb")),
        fmt_vec3(value_vec3_path(terms, "direct_rgb")),
        fmt_vec3(value_vec3_path(terms, "ambient_rgb")),
        fmt_vec3(value_vec3_path(terms, "direct_plus_ambient_rgb")),
    )
}

fn string_path(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("n/a")
        .to_owned()
}

fn value_f64_path(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn value_string_path(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn value_vec2_path(value: &Value, key: &str) -> Option<[f64; 2]> {
    let values = value.get(key)?.as_array()?;
    Some([values.first()?.as_f64()?, values.get(1)?.as_f64()?])
}

fn value_vec3_path(value: &Value, key: &str) -> Option<[f64; 3]> {
    let values = value.get(key)?.as_array()?;
    Some([
        values.first()?.as_f64()?,
        values.get(1)?.as_f64()?,
        values.get(2)?.as_f64()?,
    ])
}

fn value_rgba_path(value: &Value, key: &str) -> Option<[u64; 4]> {
    value.get(key).and_then(rgba_array)
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
    output.push_str(&format!(
        "- Browser best coverage matches Rust frontmost / expected-best / actual-best: `{}` / `{}` / `{}`\n",
        report.browser_best_coverage_matches_rust_frontmost,
        report.browser_best_coverage_matches_rust_expected_best_subpixel,
        report.browser_best_coverage_matches_rust_actual_best_subpixel
    ));
    output.push_str(&format!(
        "- Browser best sample Rust color count: `{}`; actual/expected/tie closer: `{}` / `{}` / `{}`\n",
        report.browser_best_sample_rust_color_count,
        report.browser_best_sample_actual_color_closer,
        report.browser_best_sample_expected_color_closer,
        report.browser_best_sample_color_tied
    ));
    output.push_str(&format!(
        "- Browser best sample mean actual/expected RGB distance: `{}` / `{}`\n\n",
        fmt_opt_f64(report.browser_best_sample_mean_actual_rgb_distance),
        fmt_opt_f64(report.browser_best_sample_mean_expected_rgb_distance)
    ));
    output.push_str(&format!(
        "- Browser best coverage sample Rust color count: `{}`; actual/expected/tie closer: `{}` / `{}` / `{}`\n",
        report.browser_best_coverage_sample_rust_color_count,
        report.browser_best_coverage_sample_actual_color_closer,
        report.browser_best_coverage_sample_expected_color_closer,
        report.browser_best_coverage_sample_color_tied
    ));
    output.push_str(&format!(
        "- Browser best coverage sample mean actual/expected RGB distance: `{}` / `{}`\n\n",
        fmt_opt_f64(report.browser_best_coverage_sample_mean_actual_rgb_distance),
        fmt_opt_f64(report.browser_best_coverage_sample_mean_expected_rgb_distance)
    ));
    output.push_str("## Browser Best Surface Relations\n\n");
    output.push_str("Browser best vs Rust frontmost:\n\n");
    write_counts(&mut output, &report.browser_best_to_rust_frontmost_relation);
    output.push_str("Browser best vs Rust expected-best:\n\n");
    write_counts(
        &mut output,
        &report.browser_best_to_rust_expected_best_relation,
    );
    output.push_str("Browser best vs Rust actual-best:\n\n");
    write_counts(
        &mut output,
        &report.browser_best_to_rust_actual_best_relation,
    );
    output.push_str("Browser best colors closer to expected, grouped by expected relation:\n\n");
    write_counts(
        &mut output,
        &report.browser_best_expected_closer_to_expected_relation,
    );
    output.push_str("Browser best colors closer to actual, grouped by actual relation:\n\n");
    write_counts(
        &mut output,
        &report.browser_best_actual_closer_to_actual_relation,
    );
    output.push_str("Browser best coverage vs Rust frontmost:\n\n");
    write_counts(
        &mut output,
        &report.browser_best_coverage_to_rust_frontmost_relation,
    );
    output.push_str("Browser best coverage vs Rust expected-best:\n\n");
    write_counts(
        &mut output,
        &report.browser_best_coverage_to_rust_expected_best_relation,
    );
    output.push_str("Browser best coverage vs Rust actual-best:\n\n");
    write_counts(
        &mut output,
        &report.browser_best_coverage_to_rust_actual_best_relation,
    );
    output.push_str(
        "Browser best coverage colors closer to expected, grouped by expected relation:\n\n",
    );
    write_counts(
        &mut output,
        &report.browser_best_coverage_expected_closer_to_expected_relation,
    );
    output
        .push_str("Browser best coverage colors closer to actual, grouped by actual relation:\n\n");
    write_counts(
        &mut output,
        &report.browser_best_coverage_actual_closer_to_actual_relation,
    );
    output.push_str("## Rendered To Rust Expected Best\n\n");
    write_counts(&mut output, &report.rendered_to_rust_expected_best_subpixel);
    output.push_str("## Browser Best To Rust Expected Best\n\n");
    write_counts(
        &mut output,
        &report.browser_best_to_rust_expected_best_subpixel,
    );
    output.push_str("## Browser Best Coverage To Rust Expected Best\n\n");
    write_counts(
        &mut output,
        &report.browser_best_coverage_to_rust_expected_best_subpixel,
    );
    output.push_str("## Same-Surface PBR Term vs Output Residual\n\n");
    output.push_str(
        "These rows compare source-derived Browser PBR terms with Rust CPU PBR terms only when the joined surfaces match. The output distance is the final actual-vs-three-vrm expected RGB distance at the same pixel. Diffuse/specular lobe and normal columns are optional-field subset means and show their own `n`; raw normal columns keep each side's world-space convention, while the Rust-space normal column maps only the Browser three.js normal through the observed three.js-to-Rust basis flip `[-x, y, -z]` and leaves the Rust shading normal as captured. Read the normal source-pair column before treating the basis-adjusted distance as a like-for-like normal-map comparison.\n\n",
    );
    output.push_str("| Pair | Count | Normal sources | Diffuse lobe dist | Specular lobe dist | Direct term dist | Ambient term dist | Total term dist | Browser 3js normal raw -> Rust shade | Browser wgpu normal raw -> Rust shade | Browser 3js normal Rust-space -> Rust shade | Output RGB dist |\n");
    output.push_str("|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    write_pbr_summary_row(
        &mut output,
        "Browser best -> Rust frontmost",
        &report.pbr_best_to_frontmost_term_output,
    );
    write_pbr_summary_row(
        &mut output,
        "Browser best -> Rust expected-best",
        &report.pbr_best_to_expected_term_output,
    );
    write_pbr_summary_row(
        &mut output,
        "Browser coverage -> Rust frontmost",
        &report.pbr_coverage_to_frontmost_term_output,
    );
    write_pbr_summary_row(
        &mut output,
        "Browser coverage -> Rust expected-best",
        &report.pbr_coverage_to_expected_term_output,
    );
    output.push('\n');
    output.push_str("## Top Rendered/Expected Disagreements\n\n");
    if report.top_disagreements.is_empty() {
        output.push_str("_None_\n");
    } else {
        output.push_str("| Pixel | Rendered | Browser best | Browser coverage | Rust frontmost | Rust expected-best | Relations | Samples | Output colors | Browser colors | Browser/Rust PBR terms |\n");
        output.push_str("|---|---|---|---|---|---|---|---|---|---|---|\n");
        for item in &report.top_disagreements {
            output.push_str(&format!(
                "| {},{} | {} | {} | {} area={} pts={} | {} | {} | subpixel={} coverage={} | subpixel={} coverage={} rust={} | actual={} expected={} output_dist={} | subpixel_rgba={} subpixel_actual_dist={} subpixel_expected_dist={} coverage_rgba={} coverage_actual_dist={} coverage_expected_dist={} | browser_subpixel=[{}] browser_coverage=[{}] rust_front=[{}] rust_expected=[{}] |\n",
                item.x,
                item.y,
                surface_label(item.rendered_owner.as_ref()),
                surface_label(item.browser_best_subpixel.as_ref()),
                surface_label(item.browser_best_coverage.as_ref()),
                fmt_opt_f64(item.browser_best_coverage_area_pixels),
                fmt_opt_u64(item.browser_best_coverage_point_count),
                surface_label(item.rust_frontmost.as_ref()),
                surface_label(item.rust_expected_best_subpixel.as_ref()),
                item.browser_best_to_expected_relation,
                item.browser_best_coverage_to_expected_relation,
                fmt_pair(item.browser_best_subpixel_sample),
                fmt_pair(item.browser_best_coverage_sample),
                fmt_pair(item.rust_expected_best_subpixel_sample),
                fmt_rgba(item.actual_rgba),
                fmt_rgba(item.expected_rgba),
                fmt_opt_f64(item.actual_expected_rgb_distance),
                fmt_rgba(item.browser_best_sample_cpu_base_color),
                fmt_opt_f64(item.browser_best_sample_actual_rgb_distance),
                fmt_opt_f64(item.browser_best_sample_expected_rgb_distance),
                fmt_rgba(item.browser_best_coverage_sample_cpu_base_color),
                fmt_opt_f64(item.browser_best_coverage_sample_actual_rgb_distance),
                fmt_opt_f64(item.browser_best_coverage_sample_expected_rgb_distance),
                item.browser_best_pbr_terms,
                item.browser_best_coverage_pbr_terms,
                item.rust_frontmost_pbr_terms,
                item.rust_expected_best_pbr_terms,
            ));
        }
    }
    output
}

fn write_pbr_summary_row(output: &mut String, label: &str, summary: &PbrTermOutputSummary) {
    output.push_str(&format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
        label,
        summary.count,
        fmt_counts_inline(&summary.normal_source_pairs),
        fmt_mean_with_count(
            summary.mean_diffuse_lobe_rgb_distance,
            summary.diffuse_lobe_sample_count,
        ),
        fmt_mean_with_count(
            summary.mean_specular_lobe_rgb_distance,
            summary.specular_lobe_sample_count,
        ),
        fmt_opt_f64(summary.mean_direct_rgb_distance),
        fmt_opt_f64(summary.mean_ambient_rgb_distance),
        fmt_opt_f64(summary.mean_total_rgb_distance),
        fmt_mean_with_count(
            summary.mean_shading_normal_three_js_distance,
            summary.shading_normal_three_js_sample_count,
        ),
        fmt_mean_with_count(
            summary.mean_shading_normal_wgpu_compat_distance,
            summary.shading_normal_wgpu_compat_sample_count,
        ),
        fmt_mean_with_count(
            summary.mean_shading_normal_three_js_rust_space_distance,
            summary.shading_normal_three_js_rust_space_sample_count,
        ),
        fmt_opt_f64(summary.mean_output_rgb_distance),
    ));
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

fn fmt_vec2(value: Option<[f64; 2]>) -> String {
    value.map_or_else(
        || "n/a".to_owned(),
        |value| format!("{:.6},{:.6}", value[0], value[1]),
    )
}

fn fmt_vec3(value: Option<[f64; 3]>) -> String {
    value.map_or_else(
        || "n/a".to_owned(),
        |value| format!("{:.3},{:.3},{:.3}", value[0], value[1], value[2]),
    )
}

fn fmt_rgba(value: Option<[u64; 4]>) -> String {
    value.map_or_else(
        || "n/a".to_owned(),
        |value| format!("{},{},{},{}", value[0], value[1], value[2], value[3]),
    )
}

fn fmt_opt_f64(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), |value| format!("{value:.4}"))
}

fn fmt_mean_with_count(value: Option<f64>, count: u64) -> String {
    value.map_or_else(
        || "n/a (n=0)".to_owned(),
        |value| format!("{value:.4} (n={count})"),
    )
}

fn fmt_opt_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), |value| value.to_string())
}

fn fmt_counts_inline(counts: &BTreeMap<String, u64>) -> String {
    if counts.is_empty() {
        return "n/a".to_owned();
    }
    counts
        .iter()
        .map(|(key, count)| format!("{key}: {count}"))
        .collect::<Vec<_>>()
        .join("<br>")
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
    assert_eq!(
        browser_normal_to_rust_space([0.3, 0.1, 0.94]),
        [-0.3, 0.1, -0.94]
    );
    let mut basis_accumulator = PbrTermOutputAccumulator::default();
    basis_accumulator.observe(
        Some(PbrTermSample {
            normal_source: Some("browser_geometric".to_owned()),
            diffuse_lobe_rgb: Some([0.0, 0.0, 0.0]),
            specular_lobe_rgb: Some([0.0, 0.0, 0.0]),
            direct_rgb: [0.0, 0.0, 0.0],
            ambient_rgb: [0.0, 0.0, 0.0],
            direct_plus_ambient_rgb: [0.0, 0.0, 0.0],
            shading_normal_three_js: Some([0.0, 0.0, 1.0]),
            shading_normal_wgpu_compat: None,
            shading_normal: None,
        }),
        Some(PbrTermSample {
            normal_source: Some("rust_shade".to_owned()),
            diffuse_lobe_rgb: Some([0.0, 0.0, 0.0]),
            specular_lobe_rgb: Some([0.0, 0.0, 0.0]),
            direct_rgb: [0.0, 0.0, 0.0],
            ambient_rgb: [0.0, 0.0, 0.0],
            direct_plus_ambient_rgb: [0.0, 0.0, 0.0],
            shading_normal_three_js: None,
            shading_normal_wgpu_compat: None,
            shading_normal: Some([0.0, 0.0, -1.0]),
        }),
        Some(0.0),
    );
    let basis_summary = basis_accumulator.into_summary();
    assert!(basis_summary
        .mean_shading_normal_three_js_distance
        .is_some_and(|distance| (distance - 2.0).abs() < 1e-12));
    assert!(basis_summary
        .mean_shading_normal_three_js_rust_space_distance
        .is_some_and(|distance| distance.abs() < 1e-12));
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
                        "candidate": {
                            "materialName": "body:vrm-rs-owner-id-diagnostic",
                            "triangle": 7,
                            "browserPbrTerms": {
                                "normalSource": "normal_map_sampled_missing_tangent_or_normal",
                                "diffuseLobeRgb": [0.16, 0.15, 0.15],
                                "specularLobeRgb": [0.03, 0.03, 0.03],
                                "directRgb": [0.19, 0.18, 0.18],
                                "ambientRgb": [0.01, 0.01, 0.01],
                                "directPlusAmbientRgb": [0.20, 0.19, 0.19],
                                "shadingNormalThreeJs": [0.0, 0.0, 1.0],
                                "shadingNormalWgpuCompat": [0.1, 0.0, 0.995]
                            }
                        }
                    },
                    "bestCoverage": {
                        "sampleCenter": [0.62, 0.48],
                        "coverageAreaPixels": 0.375,
                        "coveragePointCount": 4,
                        "candidate": {
                        "materialName": "body:vrm-rs-owner-id-diagnostic",
                        "triangle": 7,
                        "browserPbrTerms": {
                            "normalSource": "normal_map_sampled_missing_tangent_or_normal",
                            "diffuseLobeRgb": [0.15, 0.14, 0.14],
                            "specularLobeRgb": [0.03, 0.03, 0.02],
                            "directRgb": [0.18, 0.17, 0.17],
                            "ambientRgb": [0.01, 0.01, 0.01],
                            "directPlusAmbientRgb": [0.19, 0.18, 0.18],
                            "shadingNormalThreeJs": [0.0, 0.0, 1.0],
                            "shadingNormalWgpuCompat": [0.1, 0.0, 0.995]
                        }
                    }
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
                "actual": [10, 10, 10, 255],
                "expected": [100, 100, 100, 255],
                "frontmost_visible": {"materialName": "hair", "triangle": 3},
                "best_subpixel_visible_expected": {
                    "sample": [0.7, 0.5],
                    "candidate": {
                        "material_name": "body",
                        "triangle": 7,
                        "pbr_terms": {
                            "normal_source": "normal_map_tangent_space",
                            "diffuse_lobe_rgb": [0.15, 0.14, 0.14],
                            "specular_lobe_rgb": [0.03, 0.03, 0.02],
                            "direct_rgb": [0.18, 0.17, 0.17],
                            "ambient_rgb": [0.01, 0.01, 0.01],
                            "direct_plus_ambient_rgb": [0.19, 0.18, 0.18],
                            "shading_normal": [0.1, 0.0, 0.995]
                        }
                    }
                },
                "best_subpixel_visible_actual": {
                    "sample": [0.3, 0.5],
                    "candidate": {"material_name": "hair", "triangle": 3}
                },
                "subpixel_visible_candidates": [{
                    "sample": [0.7, 0.5],
                    "candidate": {
                        "material_name": "body",
                        "triangle": 7,
                        "cpu_base_color_rgba": [100, 100, 100, 255]
                    }
                }],
                "coverage_visible_candidates": [{
                    "sample": [0.62, 0.48],
                    "coverage_area_pixels": 0.375,
                    "coverage_point_count": 4,
                    "candidate": {
                        "material_name": "body",
                        "triangle": 7,
                        "cpu_base_color_rgba": [100, 100, 100, 255]
                    }
                }]
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
    assert_eq!(
        report.browser_best_coverage_matches_rust_expected_best_subpixel,
        1
    );
    assert_eq!(report.browser_best_sample_rust_color_count, 1);
    assert_eq!(report.browser_best_sample_expected_color_closer, 1);
    assert_eq!(report.browser_best_coverage_sample_rust_color_count, 1);
    assert_eq!(report.browser_best_coverage_sample_expected_color_closer, 1);
    assert_eq!(
        report
            .browser_best_to_rust_expected_best_relation
            .get("same-surface"),
        Some(&1)
    );
    assert_eq!(
        report
            .browser_best_coverage_to_rust_expected_best_relation
            .get("same-surface"),
        Some(&1)
    );
    assert_eq!(
        report
            .browser_best_expected_closer_to_expected_relation
            .get("same-surface"),
        Some(&1)
    );
    assert_eq!(report.pbr_best_to_frontmost_term_output.count, 0);
    assert_eq!(report.pbr_best_to_expected_term_output.count, 1);
    assert!(report
        .pbr_best_to_expected_term_output
        .mean_diffuse_lobe_rgb_distance
        .is_some_and(|distance| (distance - 0.017320508075688787).abs() < 1e-12));
    assert!(report
        .pbr_best_to_expected_term_output
        .mean_specular_lobe_rgb_distance
        .is_some_and(|distance| (distance - 0.01).abs() < 1e-12));
    assert!(report
        .pbr_best_to_expected_term_output
        .mean_direct_rgb_distance
        .is_some_and(|distance| (distance - 0.017320508075688787).abs() < 1e-12));
    assert_eq!(
        report
            .pbr_best_to_expected_term_output
            .normal_source_pairs
            .get("normal_map_sampled_missing_tangent_or_normal -> normal_map_tangent_space"),
        Some(&1)
    );
    assert!(report
        .pbr_best_to_expected_term_output
        .mean_shading_normal_wgpu_compat_distance
        .is_some_and(|distance| distance.abs() < 1e-12));
    assert!(report
        .pbr_best_to_expected_term_output
        .mean_shading_normal_three_js_distance
        .is_some_and(|distance| distance > 0.09 && distance < 0.11));
    assert!(report
        .pbr_best_to_expected_term_output
        .mean_output_rgb_distance
        .is_some_and(|distance| (distance - 155.88457268119896).abs() < 1e-12));
    assert_eq!(report.pbr_coverage_to_expected_term_output.count, 1);
    assert_eq!(
        report
            .pbr_coverage_to_expected_term_output
            .diffuse_lobe_sample_count,
        1
    );
    assert!(report
        .pbr_coverage_to_expected_term_output
        .mean_diffuse_lobe_rgb_distance
        .is_some_and(|distance| distance.abs() < 1e-12));
    assert!(report
        .pbr_coverage_to_expected_term_output
        .mean_specular_lobe_rgb_distance
        .is_some_and(|distance| distance.abs() < 1e-12));
    assert!(report
        .pbr_coverage_to_expected_term_output
        .mean_shading_normal_wgpu_compat_distance
        .is_some_and(|distance| distance.abs() < 1e-12));
    assert!(report.top_disagreements.is_empty());
    let markdown = markdown_report(&report);
    assert!(markdown.contains("Rendered owner matches Rust frontmost"));
    assert!(markdown.contains("body:tri7 -> body:tri7"));
    assert!(markdown.contains("Browser best sample Rust color count"));
    assert!(markdown.contains("Browser best coverage sample Rust color count"));
    assert!(markdown.contains("Browser Best Surface Relations"));
    assert!(markdown.contains("Same-Surface PBR Term vs Output Residual"));
    assert!(markdown.contains("Diffuse lobe dist"));
    assert!(markdown.contains("Browser 3js normal raw -> Rust shade"));
    assert!(markdown.contains("Browser 3js normal Rust-space -> Rust shade"));
    assert!(markdown.contains(
        "| Browser best -> Rust expected-best | 1 | normal_map_sampled_missing_tangent_or_normal -> normal_map_tangent_space: 1 | 0.0173 (n=1) | 0.0100 (n=1) | 0.0173"
    ));
    let legacy_browser = serde_json::json!({
        "terms": {
            "directRgb": [0.19, 0.18, 0.18],
            "ambientRgb": [0.01, 0.01, 0.01],
            "directPlusAmbientRgb": [0.20, 0.19, 0.19]
        }
    });
    let legacy_rust = serde_json::json!({
        "terms": {
            "direct_rgb": [0.18, 0.17, 0.17],
            "ambient_rgb": [0.01, 0.01, 0.01],
            "direct_plus_ambient_rgb": [0.19, 0.18, 0.18]
        }
    });
    let mut legacy_accumulator = PbrTermOutputAccumulator::default();
    legacy_accumulator.observe(
        browser_pbr_terms(&legacy_browser, "/terms"),
        rust_pbr_terms(&legacy_rust, "/terms"),
        Some(12.0),
    );
    let legacy_summary = legacy_accumulator.into_summary();
    assert_eq!(legacy_summary.count, 1);
    assert_eq!(legacy_summary.diffuse_lobe_sample_count, 0);
    assert!(legacy_summary.mean_diffuse_lobe_rgb_distance.is_none());
    assert!(legacy_summary
        .mean_shading_normal_three_js_distance
        .is_none());
    assert!(legacy_summary
        .mean_direct_rgb_distance
        .is_some_and(|distance| (distance - 0.017320508075688787).abs() < 1e-12));
    let pbr_fixture = serde_json::json!({
        "browserPbrTerms": {
            "model": "MeshStandardMaterial",
            "nDotL": 0.7,
            "nDotV": 0.9,
            "normalSource": "normal_map_tangent_space",
            "normalUv": [0.25, 0.75],
            "normalTextureRgba": [127, 123, 255, 255],
            "baseUv": [0.3, 0.4],
            "baseTextureRgba": [60, 55, 55, 255],
            "geometricNormal": [0.0, 0.0, 1.0],
            "shadingNormalThreeJs": [0.1, -0.1, 0.99],
            "shadingNormalWgpuCompat": [0.1, 0.1, 0.99],
            "tangentSpaceNormalThreeJs": [0.0, -0.03, 1.0],
            "tangentSpaceNormalWgpuCompat": [0.0, 0.03, 1.0],
            "diffuseLobeRgb": [0.16, 0.15, 0.15],
            "specularLobeRgb": [0.03, 0.03, 0.03],
            "directRgb": [0.19, 0.18, 0.18],
            "ambientRgb": [0.01, 0.01, 0.01],
            "directPlusAmbientRgb": [0.20, 0.19, 0.19],
            "specularF90": 1.0,
            "roughnessUsed": 0.657,
            "metalness": 0.0,
            "roughness": 0.657
        },
        "rust": {
            "pbr_terms": {
                "n_dot_l": 0.7,
                "n_dot_v": 0.9,
                "normal_source": "normal_map_tangent_space",
                "normal_uv": [0.25, 0.75],
                "normal_texture_rgba": [127, 123, 255, 255],
                "geometric_normal": [0.0, 0.0, 1.0],
                "shading_normal": [0.1, 0.1, 0.99],
                "tangent_space_normal_three_js": [0.0, -0.03, 1.0],
                "tangent_space_normal_wgpu_compat": [0.0, 0.03, 1.0],
                "diffuse_lobe_rgb": [0.16, 0.15, 0.15],
                "specular_lobe_rgb": [0.02, 0.02, 0.02],
                "direct_rgb": [0.18, 0.17, 0.17],
                "ambient_rgb": [0.01, 0.01, 0.01],
                "direct_plus_ambient_rgb": [0.19, 0.18, 0.18]
            }
        }
    });
    assert!(browser_pbr_terms_summary(&pbr_fixture, "/browserPbrTerms")
        .contains("shade-wgpu=0.100,0.100,0.990"));
    assert!(browser_pbr_terms_summary(&pbr_fixture, "/browserPbrTerms")
        .contains("direct=0.190,0.180,0.180"));
    assert!(browser_pbr_terms_summary(&pbr_fixture, "/browserPbrTerms").contains("f90=1.0000"));
    assert!(
        rust_pbr_terms_summary(&pbr_fixture, "/rust/pbr_terms").contains("total=0.190,0.180,0.180")
    );
    assert!(rust_pbr_terms_summary(&pbr_fixture, "/rust/pbr_terms")
        .contains("tan-wgpu=0.000,0.030,1.000"));
    Ok(())
}
