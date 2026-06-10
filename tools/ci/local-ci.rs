#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
image = { version = "0.25.10", default-features = false, features = ["png"] }
serde_json = "1.0.150"
---

//! Local replacement for the removed GitHub Actions workflow.
//!
//! Usage:
//! cargo +nightly -Zscript tools/ci/local-ci.rs
//! cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures
//! cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity

use clap::{Parser, ValueEnum};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

const THREE_VRM_COMMIT: &str = "9d125586f6d7da094b0ac5f204cebf19586f2397";
const THREE_VRM_VIEWER_COMMIT: &str = "75ab65c9d4e488521d41bff7f5cfd1976a0b16e8";
const VRM_SPEC_COMMIT: &str = "3942748efbc803b258e288e0f6c993c6bb96cebf";

fn main() {
    if let Err(err) = run(Options::parse_from(script_args())) {
        eprintln!("local-ci failed: {err}");
        std::process::exit(1);
    }
}

fn script_args() -> impl Iterator<Item = OsString> {
    env::args_os().filter(|arg| arg != "--")
}

#[derive(Clone, Debug, Parser)]
#[command(
    name = "local-ci",
    about = "Local vrm-rs CI runner",
    after_help = "Examples:\n  cargo +nightly -Zscript tools/ci/local-ci.rs\n  cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures\n  cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity"
)]
struct Options {
    #[arg(long)]
    external_fixtures: bool,
    #[arg(long)]
    render_parity: bool,
    #[arg(long)]
    skip_core: bool,
    #[arg(long)]
    skip_coverage: bool,
    #[arg(long)]
    skip_download: bool,
    #[arg(long)]
    skip_three_vrm_build: bool,
    #[arg(long)]
    skip_golden_generation: bool,
    #[arg(long)]
    skip_playwright_install: bool,
    #[arg(long, default_value_t = 256)]
    render_width: u32,
    #[arg(long, default_value_t = 256)]
    render_height: u32,
    #[arg(long, default_value_t = 3.0)]
    render_camera_z: f32,
    #[arg(long, default_value_t = 0.78)]
    render_mtoon_exposure: f32,
    #[arg(long, default_value_t = 0.12)]
    render_mtoon_ambient_base: f32,
    #[arg(long, default_value_t = 0.20)]
    render_mtoon_ambient_gi_scale: f32,
    #[arg(long, default_value_t = 0.03183099)]
    render_pbr_ambient: f32,
    #[arg(long, default_value_t = 1.0)]
    render_direct_light_scale: f32,
    #[arg(long, default_value_t = std::f32::consts::PI)]
    render_three_vrm_directional_intensity: f32,
    #[arg(long, default_value_t = 1.0)]
    render_three_vrm_directional_x: f32,
    #[arg(long, default_value_t = 1.0)]
    render_three_vrm_directional_y: f32,
    #[arg(long, default_value_t = 1.0)]
    render_three_vrm_directional_z: f32,
    #[arg(long, default_value_t = 1.0)]
    render_directional_r: f32,
    #[arg(long, default_value_t = 1.0)]
    render_directional_g: f32,
    #[arg(long, default_value_t = 1.0)]
    render_directional_b: f32,
    #[arg(long, default_value_t = 0.1)]
    render_three_vrm_ambient_intensity: f32,
    #[arg(long, value_enum, default_value_t = RenderMtoonLightAccumulation::ThreeVrm)]
    render_mtoon_light_accumulation: RenderMtoonLightAccumulation,
    #[arg(long)]
    render_sync_three_vrm_light_units: bool,
    #[arg(long, default_value_t = 0.0)]
    render_mtoon_time: f32,
    #[arg(long = "render-expression")]
    render_expressions: Vec<String>,
    #[arg(long)]
    render_fail_under: Option<f32>,
    #[arg(long)]
    render_max_selected_channel_delta: Option<u8>,
    #[arg(long)]
    render_max_alpha_delta: Option<u8>,
    #[arg(long, value_enum, default_value_t = RenderPsnrMetric::RgbVisible)]
    render_psnr_metric: RenderPsnrMetric,
    #[arg(long, default_value_t = 128)]
    render_alpha_mismatch_tolerance: usize,
    #[arg(long, default_value_t = 0)]
    render_alpha_channel_tolerance: u8,
    #[arg(long, value_enum, default_value_t = RenderBackground::OpaqueBlack)]
    render_background: RenderBackground,
    #[arg(long)]
    render_disable_outlines: bool,
    #[arg(long, default_value_t = 1.0)]
    render_outline_width_scale: f32,
    #[arg(long)]
    render_disable_normal_maps: bool,
    #[arg(long)]
    render_disable_texture_mips: bool,
    #[arg(long, value_enum, default_value_t = RenderNormalMapMode::GeneratedTangents)]
    render_normal_map_mode: RenderNormalMapMode,
    #[arg(long, default_value_t = 1.0)]
    render_normal_map_scale: f32,
    #[arg(long, value_enum, default_value_t = RenderDiagnosticMode::Shaded)]
    render_diagnostic_mode: RenderDiagnosticMode,
    #[arg(long = "render-fixture")]
    render_fixtures: Vec<String>,
    #[arg(long, default_value = ".external-fixtures/official")]
    fixture_dir: PathBuf,
    #[arg(long, default_value = ".external-fixtures/golden")]
    golden_dir: PathBuf,
    #[arg(long, default_value = ".external-fixtures/three-vrm")]
    three_vrm_root: PathBuf,
    #[arg(long, default_value = ".external-fixtures/render-parity")]
    render_parity_dir: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RenderBackground {
    OpaqueBlack,
    Transparent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RenderPsnrMetric {
    Rgba,
    RgbAll,
    RgbOpaque,
    RgbVisible,
    RgbNonblack,
    RgbInterior1px,
    RgbVisibleInterior1px,
    RgbNonblackInterior1px,
    RgbSharedNonblackInterior1px,
    RgbSharedNonblackInterior2px,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RenderMtoonLightAccumulation {
    Tuned,
    ThreeVrm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RenderNormalMapMode {
    GeneratedTangents,
    Derivative,
    ViewDerivative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RenderDiagnosticMode {
    Shaded,
    Flat,
    BaseFactor,
    BaseColor,
    BaseColorFlipV,
    Uv,
}

impl RenderMtoonLightAccumulation {
    fn as_cli_value(self) -> &'static str {
        match self {
            Self::Tuned => "tuned",
            Self::ThreeVrm => "three-vrm",
        }
    }
}

impl RenderNormalMapMode {
    fn as_cli_value(self) -> &'static str {
        match self {
            Self::GeneratedTangents => "generated-tangents",
            Self::Derivative => "derivative",
            Self::ViewDerivative => "view-derivative",
        }
    }
}

impl RenderDiagnosticMode {
    fn as_cli_value(self) -> &'static str {
        match self {
            Self::Shaded => "shaded",
            Self::Flat => "flat",
            Self::BaseFactor => "base-factor",
            Self::BaseColor => "base-color",
            Self::BaseColorFlipV => "base-color-flip-v",
            Self::Uv => "uv",
        }
    }
}

impl RenderPsnrMetric {
    fn as_cli_value(self) -> &'static str {
        match self {
            Self::Rgba => "rgba",
            Self::RgbAll => "rgb-all",
            Self::RgbOpaque => "rgb-opaque",
            Self::RgbVisible => "rgb-visible",
            Self::RgbNonblack => "rgb-nonblack",
            Self::RgbInterior1px => "rgb-interior1px",
            Self::RgbVisibleInterior1px => "rgb-visible-interior1px",
            Self::RgbNonblackInterior1px => "rgb-nonblack-interior1px",
            Self::RgbSharedNonblackInterior1px => "rgb-shared-nonblack-interior1px",
            Self::RgbSharedNonblackInterior2px => "rgb-shared-nonblack-interior2px",
        }
    }
}

impl RenderBackground {
    fn as_cli_value(self) -> &'static str {
        match self {
            Self::OpaqueBlack => "opaque-black",
            Self::Transparent => "transparent",
        }
    }
}

fn run(options: Options) -> Result<(), String> {
    ensure_no_github_actions_workflows()?;

    if !options.skip_core {
        run_cmd("cargo", ["fmt", "--all", "--", "--check"])?;
        run_cmd("cargo", ["test", "--workspace", "--all-features"])?;
        run_cmd(
            "cargo",
            [
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ],
        )?;
        run_example_smokes()?;
    }

    if !options.skip_coverage {
        run_cmd(
            "cargo",
            [
                "llvm-cov",
                "--workspace",
                "--all-features",
                "--summary-only",
                "--fail-under-lines",
                "70",
            ],
        )?;
    }

    if options.external_fixtures {
        run_external_fixture_ci(&options)?;
    }
    if options.render_parity {
        run_render_parity_ci(&options)?;
    }

    Ok(())
}

fn run_example_smokes() -> Result<(), String> {
    for example in [
        "mtoon_renderer_skeletons",
        "bevy_mtoon_materialization",
        "custom_engine_adapter",
    ] {
        run_cmd("cargo", ["run", "--example", example])?;
    }
    Ok(())
}

fn ensure_no_github_actions_workflows() -> Result<(), String> {
    let workflows = Path::new(".github").join("workflows");
    if !workflows.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(&workflows)
        .map_err(|err| format!("failed to read {}: {err}", path(&workflows)))?;
    let workflow_files = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("yml") || extension.eq_ignore_ascii_case("yaml")
                })
        })
        .collect::<Vec<_>>();

    if workflow_files.is_empty() {
        Ok(())
    } else {
        let files = workflow_files
            .iter()
            .map(|file| format!("  - {}", path(file)))
            .collect::<Vec<_>>()
            .join("\n");
        Err(format!(
            "GitHub Actions workflows are intentionally not used in this repository. \
             Remove these files and use `cargo +nightly -Zscript tools/ci/local-ci.rs` instead:\n{files}"
        ))
    }
}

fn prepare_external_inputs(options: &Options) -> Result<(), String> {
    std::fs::create_dir_all(&options.fixture_dir).map_err(|err| err.to_string())?;
    std::fs::create_dir_all(&options.golden_dir).map_err(|err| err.to_string())?;

    if !options.skip_download {
        download_external_fixtures(options)?;
    }
    if !options.skip_three_vrm_build {
        build_three_vrm(&options.three_vrm_root)?;
    }
    Ok(())
}

fn run_external_fixture_ci(options: &Options) -> Result<(), String> {
    prepare_external_inputs(options)?;
    if !options.skip_golden_generation {
        generate_goldens(options)?;
    }
    run_external_fixture_tests(options)
}

fn run_render_parity_ci(options: &Options) -> Result<(), String> {
    prepare_external_inputs(options)?;
    if !options.skip_playwright_install {
        run_cmd("npm", ["install", "--no-save", "playwright"])?;
    }
    prepare_render_output_dirs(options)?;
    let fixtures = render_fixtures(options)?;
    for fixture in &fixtures {
        capture_three_vrm_reference(options, fixture)?;
        verify_render_imqraw_matches_rgba(options, fixture, "three-vrm")?;
        write_render_png_from_artifact(options, fixture, "three-vrm")?;
        capture_wgpu(options, fixture)?;
        verify_render_imqraw_matches_rgba(options, fixture, "wgpu")?;
        write_render_png_from_artifact(options, fixture, "wgpu")?;
        capture_bevy(options, fixture)?;
        verify_render_imqraw_matches_rgba(options, fixture, "bevy")?;
        write_render_png_from_artifact(options, fixture, "bevy")?;
        verify_render_alpha_consistency(options, fixture)?;
        compare_render_imqraw_pair(options, fixture, "wgpu")?;
        compare_render_rgba_json_pair(options, fixture, "wgpu")?;
        compare_render_imqraw_pair(options, fixture, "bevy")?;
        compare_render_rgba_json_pair(options, fixture, "bevy")?;
        write_render_diff_image(options, fixture, "wgpu")?;
        write_render_diff_image(options, fixture, "bevy")?;
    }
    let summary = render_summary_markdown(options, &fixtures)?;
    write_render_summary(options, &summary)?;
    write_render_review_manifest(options, &fixtures)?;
    write_render_visual_review(options, &fixtures, &summary)?;
    validate_render_review_manifest(options)
}

fn prepare_render_output_dirs(options: &Options) -> Result<(), String> {
    std::fs::create_dir_all(&options.render_parity_dir).map_err(|err| err.to_string())?;

    for child in ["three-vrm", "wgpu", "bevy", "reports", "diff"] {
        let dir = options.render_parity_dir.join(child);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|err| {
                format!(
                    "failed to remove stale render artifacts {}: {err}",
                    path(&dir)
                )
            })?;
        }
        std::fs::create_dir_all(&dir)
            .map_err(|err| format!("failed to create render artifact dir {}: {err}", path(&dir)))?;
    }

    let review = options.render_parity_dir.join("visual-review.html");
    if review.exists() {
        std::fs::remove_file(&review)
            .map_err(|err| format!("failed to remove stale {}: {err}", path(&review)))?;
    }
    let summary = options.render_parity_dir.join("summary.md");
    if summary.exists() {
        std::fs::remove_file(&summary)
            .map_err(|err| format!("failed to remove stale {}: {err}", path(&summary)))?;
    }
    let manifest = render_review_manifest_path(options);
    if manifest.exists() {
        std::fs::remove_file(&manifest)
            .map_err(|err| format!("failed to remove stale {}: {err}", path(&manifest)))?;
    }

    Ok(())
}

fn download_external_fixtures(options: &Options) -> Result<(), String> {
    let fixtures = [
        (
            format!(
                "https://raw.githubusercontent.com/vrm-c/vrm-specification/{VRM_SPEC_COMMIT}/samples/Seed-san/vrm/Seed-san.vrm"
            ),
            options.fixture_dir.join("Seed-san.vrm"),
        ),
        (
            format!(
                "https://raw.githubusercontent.com/vrm-c/vrm-specification/{VRM_SPEC_COMMIT}/samples/VRM1_Constraint_Twist_Sample/vrm/VRM1_Constraint_Twist_Sample.vrm"
            ),
            options.fixture_dir.join("VRM1_Constraint_Twist_Sample.vrm"),
        ),
        (
            format!(
                "https://raw.githubusercontent.com/vrm-c/vrm-specification/{VRM_SPEC_COMMIT}/samples/VRMC_materials_mtoon_UV_Animation_Test/vrm/VRMC_materials_mtoon_UV_Animation_Test.vrm"
            ),
            options
                .fixture_dir
                .join("VRMC_materials_mtoon_UV_Animation_Test.vrm"),
        ),
        (
            format!(
                "https://raw.githubusercontent.com/vrm-c/vrm-specification/{VRM_SPEC_COMMIT}/samples/VRMC_vrm_expressions_isBinary_Overridden/vrm/VRMC_vrm_expressions_isBinary_Overridden.vrm"
            ),
            options
                .fixture_dir
                .join("VRMC_vrm_expressions_isBinary_Overridden.vrm"),
        ),
        (
            format!(
                "https://raw.githubusercontent.com/vrm-c/vrm-specification/{VRM_SPEC_COMMIT}/samples/VRMC_vrm_expressions_isBinary_Overrides/vrm/VRMC_vrm_expressions_isBinary_Overrides.vrm"
            ),
            options
                .fixture_dir
                .join("VRMC_vrm_expressions_isBinary_Overrides.vrm"),
        ),
        (
            format!(
                "https://raw.githubusercontent.com/pixiv/three-vrm/{THREE_VRM_COMMIT}/packages/three-vrm-animation/examples/models/test.vrma"
            ),
            options.fixture_dir.join("test.vrma"),
        ),
        (
            format!(
                "https://raw.githubusercontent.com/pixiv/three-vrm/{THREE_VRM_VIEWER_COMMIT}/packages/vrm-viewer/examples/models/idle_loop.vrma"
            ),
            options.fixture_dir.join("idle_loop.vrma"),
        ),
    ];

    fixtures
        .iter()
        .try_for_each(|(url, out)| run_cmd("curl", ["-fL", url.as_str(), "-o", path(out).as_str()]))
}

fn build_three_vrm(root: &PathBuf) -> Result<(), String> {
    if !root.join(".git").exists() {
        run_cmd("git", ["init", path(root).as_str()])?;
        run_cmd_in(
            root,
            "git",
            [
                "remote",
                "add",
                "origin",
                "https://github.com/pixiv/three-vrm.git",
            ],
        )?;
    }
    run_cmd_in(
        root,
        "git",
        ["fetch", "--depth", "1", "origin", THREE_VRM_COMMIT],
    )?;
    run_cmd_in(root, "git", ["checkout", "--detach", "FETCH_HEAD"])?;
    run_cmd("corepack", ["enable"])?;
    run_cmd("corepack", ["prepare", "pnpm@10.24.0", "--activate"])?;
    run_cmd(
        "pnpm",
        ["-C", path(root).as_str(), "install", "--frozen-lockfile"],
    )?;
    run_cmd(
        "pnpm",
        [
            "-C",
            path(root).as_str(),
            "--filter",
            "@pixiv/three-vrm-springbone",
            "--filter",
            "@pixiv/three-vrm-core",
            "--filter",
            "@pixiv/three-vrm-materials-mtoon",
            "--filter",
            "@pixiv/three-vrm-materials-hdr-emissive-multiplier",
            "--filter",
            "@pixiv/three-vrm-materials-v0compat",
            "--filter",
            "@pixiv/three-vrm-node-constraint",
            "--filter",
            "@pixiv/three-vrm",
            "--filter",
            "@pixiv/three-vrm-animation",
            "build",
        ],
    )
}

fn generate_goldens(options: &Options) -> Result<(), String> {
    let fixture = |name: &str| path(&options.fixture_dir.join(name));
    let golden = |name: &str| path(&options.golden_dir.join(name));
    let three_vrm_root = path(&options.three_vrm_root);

    run_cmd(
        "node",
        [
            "tools/three-vrm-golden.mjs",
            "--fixture",
            fixture("Seed-san.vrm").as_str(),
            "--three-vrm-root",
            three_vrm_root.as_str(),
            "--frames",
            "8",
            "--out",
            golden("Seed-san.spring.json").as_str(),
        ],
    )?;
    run_cmd(
        "node",
        [
            "tools/three-vrm-golden.mjs",
            "--fixture",
            fixture("VRM1_Constraint_Twist_Sample.vrm").as_str(),
            "--three-vrm-root",
            three_vrm_root.as_str(),
            "--frames",
            "8",
            "--out",
            golden("VRM1_Constraint_Twist_Sample.spring.json").as_str(),
        ],
    )?;
    run_cmd(
        "node",
        [
            "tools/three-vrm-constraint-golden.mjs",
            "--fixture",
            fixture("VRM1_Constraint_Twist_Sample.vrm").as_str(),
            "--three-vrm-root",
            three_vrm_root.as_str(),
            "--out",
            golden("VRM1_Constraint_Twist_Sample.constraint.json").as_str(),
        ],
    )?;
    run_vrma_golden(
        &fixture("test.vrma"),
        &golden("Seed-san.test-vrma.json"),
        "0,0.5,1",
        options,
    )?;
    run_vrma_golden(
        &fixture("test.vrma"),
        &golden("Seed-san.test-vrma-dense.json"),
        "0,0.125,0.25,0.375,0.5,0.625,0.75,0.875,1",
        options,
    )?;
    run_vrma_golden(
        &fixture("idle_loop.vrma"),
        &golden("Seed-san.idle-loop-vrma.json"),
        "0,0.5,1",
        options,
    )
}

fn run_vrma_golden(vrma: &str, out: &str, times: &str, options: &Options) -> Result<(), String> {
    run_cmd(
        "node",
        [
            "tools/three-vrm-vrma-golden.mjs",
            "--fixture",
            path(&options.fixture_dir.join("Seed-san.vrm")).as_str(),
            "--vrma",
            vrma,
            "--three-vrm-root",
            path(&options.three_vrm_root).as_str(),
            "--times",
            times,
            "--out",
            out,
        ],
    )
}

fn run_external_fixture_tests(options: &Options) -> Result<(), String> {
    run_cargo_test_with_env(
        [("VRM_RS_FIXTURE_DIR", path(&options.fixture_dir))],
        [
            "test",
            "-p",
            "vrm-io",
            "tests::loads_external_fixture_directory",
            "--",
            "--ignored",
            "--exact",
        ],
    )?;
    run_cargo_test_with_env(
        [(
            "VRM_RS_THREE_VRM_GOLDEN",
            path(&options.golden_dir.join("Seed-san.spring.json")),
        )],
        [
            "test",
            "-p",
            "vrm-adapter",
            "tests::spring_parity_matches_three_vrm_golden_rotations",
            "--",
            "--ignored",
            "--exact",
        ],
    )?;
    run_cargo_test_with_env(
        [("VRM_RS_THREE_VRM_GOLDEN_DIR", path(&options.golden_dir))],
        [
            "test",
            "-p",
            "vrm-adapter",
            "tests::spring_parity_matches_three_vrm_golden_directory",
            "--",
            "--ignored",
            "--exact",
        ],
    )?;
    run_cargo_test_with_env(
        [(
            "VRM_RS_THREE_VRM_CONSTRAINT_GOLDEN",
            path(
                &options
                    .golden_dir
                    .join("VRM1_Constraint_Twist_Sample.constraint.json"),
            ),
        )],
        [
            "test",
            "-p",
            "vrm-adapter",
            "tests::node_constraint_manager_matches_three_vrm_golden",
            "--",
            "--ignored",
            "--exact",
        ],
    )?;
    run_cargo_test_with_env(
        [(
            "VRM_RS_THREE_VRM_VRMA_GOLDEN",
            path(&options.golden_dir.join("Seed-san.test-vrma.json")),
        )],
        [
            "test",
            "-p",
            "vrm-adapter",
            "tests::vrma_application_matches_three_vrm_golden",
            "--",
            "--ignored",
            "--exact",
        ],
    )?;
    run_cargo_test_with_env(
        [(
            "VRM_RS_THREE_VRM_VRMA_GOLDEN_DIR",
            path(&options.golden_dir),
        )],
        [
            "test",
            "-p",
            "vrm-adapter",
            "tests::vrma_application_matches_three_vrm_golden_directory",
            "--",
            "--ignored",
            "--exact",
        ],
    )
}

#[derive(Clone, Debug)]
struct RenderFixture {
    name: String,
    stem: String,
    path: PathBuf,
}

fn render_fixtures(options: &Options) -> Result<Vec<RenderFixture>, String> {
    let names = if options.render_fixtures.is_empty() {
        vec!["Seed-san.vrm".to_owned()]
    } else {
        options.render_fixtures.clone()
    };
    names
        .into_iter()
        .map(|name| {
            let fixture_path = PathBuf::from(&name);
            let path = if fixture_path.is_absolute() || fixture_path.components().count() > 1 {
                fixture_path
            } else {
                options.fixture_dir.join(&name)
            };
            if !path.exists() {
                return Err(format!(
                    "render fixture does not exist: {}",
                    self::path(&path)
                ));
            }
            let stem = path.file_stem().and_then(OsStr::to_str).ok_or_else(|| {
                format!(
                    "render fixture has no valid file stem: {}",
                    self::path(&path)
                )
            })?;
            Ok(RenderFixture {
                name,
                stem: sanitize_artifact_stem(stem),
                path,
            })
        })
        .collect()
}

fn sanitize_artifact_stem(stem: &str) -> String {
    stem.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn capture_three_vrm_reference(options: &Options, fixture: &RenderFixture) -> Result<(), String> {
    let mut command = Command::new("node");
    command
        .arg("tools/render-parity/three-vrm-browser-capture.mjs")
        .arg("--fixture")
        .arg(path(&fixture.path))
        .arg("--three-vrm-root")
        .arg(path(&options.three_vrm_root))
        .arg("--out")
        .arg(path(&render_artifact(options, fixture, "three-vrm")))
        .arg("--imqraw-out")
        .arg(path(&render_imqraw_artifact(options, fixture, "three-vrm")))
        .arg("--width")
        .arg(options.render_width.to_string())
        .arg("--height")
        .arg(options.render_height.to_string())
        .arg("--background")
        .arg(options.render_background.as_cli_value())
        .arg("--camera-z")
        .arg(options.render_camera_z.to_string())
        .arg("--directional-intensity")
        .arg(options.render_three_vrm_directional_intensity.to_string())
        .arg("--directional-x")
        .arg(options.render_three_vrm_directional_x.to_string())
        .arg("--directional-y")
        .arg(options.render_three_vrm_directional_y.to_string())
        .arg("--directional-z")
        .arg(options.render_three_vrm_directional_z.to_string())
        .arg("--directional-r")
        .arg(options.render_directional_r.to_string())
        .arg("--directional-g")
        .arg(options.render_directional_g.to_string())
        .arg("--directional-b")
        .arg(options.render_directional_b.to_string())
        .arg("--ambient-intensity")
        .arg(options.render_three_vrm_ambient_intensity.to_string())
        .arg("--mtoon-time")
        .arg(options.render_mtoon_time.to_string())
        .arg("--diagnostic-render")
        .arg(options.render_diagnostic_mode.as_cli_value());
    if options.render_disable_outlines {
        command.arg("--disable-outlines");
    }
    if options.render_disable_normal_maps {
        command.arg("--disable-normal-maps");
    }
    if options.render_disable_texture_mips {
        command.arg("--disable-texture-mips");
    }
    for expression in &options.render_expressions {
        command.arg("--expression").arg(expression);
    }
    run_command(command)
}

fn capture_wgpu(options: &Options, fixture: &RenderFixture) -> Result<(), String> {
    let light_units = render_light_units(options);
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--example")
        .arg("wgpu_render_capture")
        .arg("--")
        .arg("--fixture")
        .arg(path(&fixture.path))
        .arg("--out")
        .arg(path(&render_artifact(options, fixture, "wgpu")))
        .arg("--imqraw-out")
        .arg(path(&render_imqraw_artifact(options, fixture, "wgpu")))
        .arg("--width")
        .arg(options.render_width.to_string())
        .arg("--height")
        .arg(options.render_height.to_string())
        .arg("--camera-z")
        .arg(options.render_camera_z.to_string())
        .arg("--mtoon-exposure")
        .arg(options.render_mtoon_exposure.to_string())
        .arg("--mtoon-ambient-base")
        .arg(options.render_mtoon_ambient_base.to_string())
        .arg("--mtoon-ambient-gi-scale")
        .arg(options.render_mtoon_ambient_gi_scale.to_string())
        .arg("--pbr-ambient")
        .arg(light_units.pbr_ambient.to_string())
        .arg("--direct-light-scale")
        .arg(light_units.direct_light_scale.to_string())
        .arg("--directional-r")
        .arg(options.render_directional_r.to_string())
        .arg("--directional-g")
        .arg(options.render_directional_g.to_string())
        .arg("--directional-b")
        .arg(options.render_directional_b.to_string())
        .arg("--mtoon-light-accumulation")
        .arg(options.render_mtoon_light_accumulation.as_cli_value())
        .arg("--mtoon-time")
        .arg(options.render_mtoon_time.to_string())
        .arg("--background")
        .arg(options.render_background.as_cli_value())
        .arg("--diagnostic-render")
        .arg(options.render_diagnostic_mode.as_cli_value());
    if options.render_disable_outlines {
        command.arg("--disable-outlines");
    }
    command
        .arg("--outline-width-scale")
        .arg(options.render_outline_width_scale.to_string());
    if options.render_disable_normal_maps {
        command.arg("--disable-normal-maps");
    }
    if options.render_disable_texture_mips {
        command.arg("--disable-texture-mips");
    }
    command
        .arg("--normal-map-mode")
        .arg(options.render_normal_map_mode.as_cli_value())
        .arg("--normal-map-scale")
        .arg(options.render_normal_map_scale.to_string());
    for expression in &options.render_expressions {
        command.arg("--expression").arg(expression);
    }
    run_command(command)
}

fn capture_bevy(options: &Options, fixture: &RenderFixture) -> Result<(), String> {
    let light_units = render_light_units(options);
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--example")
        .arg("bevy_render_capture")
        .arg("--")
        .arg("--fixture")
        .arg(path(&fixture.path))
        .arg("--out")
        .arg(path(&render_artifact(options, fixture, "bevy")))
        .arg("--imqraw-out")
        .arg(path(&render_imqraw_artifact(options, fixture, "bevy")))
        .arg("--width")
        .arg(options.render_width.to_string())
        .arg("--height")
        .arg(options.render_height.to_string())
        .arg("--camera-z")
        .arg(options.render_camera_z.to_string())
        .arg("--mtoon-exposure")
        .arg(options.render_mtoon_exposure.to_string())
        .arg("--mtoon-ambient-base")
        .arg(options.render_mtoon_ambient_base.to_string())
        .arg("--mtoon-ambient-gi-scale")
        .arg(options.render_mtoon_ambient_gi_scale.to_string())
        .arg("--pbr-ambient")
        .arg(light_units.pbr_ambient.to_string())
        .arg("--direct-light-scale")
        .arg(light_units.direct_light_scale.to_string())
        .arg("--directional-r")
        .arg(options.render_directional_r.to_string())
        .arg("--directional-g")
        .arg(options.render_directional_g.to_string())
        .arg("--directional-b")
        .arg(options.render_directional_b.to_string())
        .arg("--mtoon-light-accumulation")
        .arg(options.render_mtoon_light_accumulation.as_cli_value())
        .arg("--mtoon-time")
        .arg(options.render_mtoon_time.to_string())
        .arg("--background")
        .arg(options.render_background.as_cli_value())
        .arg("--diagnostic-render")
        .arg(options.render_diagnostic_mode.as_cli_value());
    if options.render_disable_outlines {
        command.arg("--disable-outlines");
    }
    command
        .arg("--outline-width-scale")
        .arg(options.render_outline_width_scale.to_string());
    if options.render_disable_normal_maps {
        command.arg("--disable-normal-maps");
    }
    if options.render_disable_texture_mips {
        command.arg("--disable-texture-mips");
    }
    command
        .arg("--normal-map-mode")
        .arg(options.render_normal_map_mode.as_cli_value())
        .arg("--normal-map-scale")
        .arg(options.render_normal_map_scale.to_string());
    for expression in &options.render_expressions {
        command.arg("--expression").arg(expression);
    }
    run_command(command)
}

#[derive(Clone, Copy, Debug)]
struct RenderLightUnits {
    direct_light_scale: f32,
    pbr_ambient: f32,
}

fn render_light_units(options: &Options) -> RenderLightUnits {
    if options.render_sync_three_vrm_light_units {
        RenderLightUnits {
            direct_light_scale: options.render_three_vrm_directional_intensity
                / std::f32::consts::PI,
            pbr_ambient: options.render_three_vrm_ambient_intensity / std::f32::consts::PI,
        }
    } else {
        RenderLightUnits {
            direct_light_scale: options.render_direct_light_scale,
            pbr_ambient: options.render_pbr_ambient,
        }
    }
}

fn compare_render_rgba_json_pair(
    options: &Options,
    fixture: &RenderFixture,
    renderer: &str,
) -> Result<(), String> {
    let mut command = Command::new("node");
    command.args([
        "tools/render-parity/compare-psnr.mjs",
        "--expected",
        path(&render_artifact(options, fixture, "three-vrm")).as_str(),
        "--actual",
        path(&render_artifact(options, fixture, renderer)).as_str(),
        "--out",
        path(&render_report(options, fixture, renderer)).as_str(),
        "--metric",
        options.render_psnr_metric.as_cli_value(),
    ]);
    run_command(command)
}

fn validate_render_review_manifest(options: &Options) -> Result<(), String> {
    let mut command = Command::new("cargo");
    command.args([
        "+nightly",
        "-Zscript",
        "tools/render-parity/validate-review-manifest.rs",
        "--manifest",
        path(&render_review_manifest_path(options)).as_str(),
    ]);
    run_command(command)
}

fn verify_render_imqraw_matches_rgba(
    options: &Options,
    fixture: &RenderFixture,
    renderer: &str,
) -> Result<(), String> {
    let mut command = Command::new("cargo");
    command.args([
        "+nightly",
        "-Zscript",
        "tools/render-parity/verify-imqraw-rgba.rs",
        "--imqraw",
        path(&render_imqraw_artifact(options, fixture, renderer)).as_str(),
        "--rgba-json",
        path(&render_artifact(options, fixture, renderer)).as_str(),
    ]);
    run_command(command)
}

fn compare_render_imqraw_pair(
    options: &Options,
    fixture: &RenderFixture,
    renderer: &str,
) -> Result<(), String> {
    let mut command = Command::new("cargo");
    command.args([
        "+nightly",
        "-Zscript",
        "tools/render-parity/compare-imqraw.rs",
        "--expected",
        path(&render_imqraw_artifact(options, fixture, "three-vrm")).as_str(),
        "--actual",
        path(&render_imqraw_artifact(options, fixture, renderer)).as_str(),
        "--out",
        path(&render_imqraw_report(options, fixture, renderer)).as_str(),
        "--metric",
        options.render_psnr_metric.as_cli_value(),
    ]);
    if let Some(fail_under) = options.render_fail_under {
        command.args(["--fail-under", fail_under.to_string().as_str()]);
    }
    if let Some(max_delta) = options.render_max_selected_channel_delta {
        command.args([
            "--max-selected-channel-delta",
            max_delta.to_string().as_str(),
        ]);
    }
    if let Some(max_delta) = options.render_max_alpha_delta {
        command.args(["--max-alpha-delta", max_delta.to_string().as_str()]);
    }
    run_command(command)
}

fn render_artifact(options: &Options, fixture: &RenderFixture, renderer: &str) -> PathBuf {
    options
        .render_parity_dir
        .join(renderer)
        .join(format!("{}.frame000.rgba.json", fixture.stem))
}

fn render_png(options: &Options, fixture: &RenderFixture, renderer: &str) -> PathBuf {
    options
        .render_parity_dir
        .join(renderer)
        .join(format!("{}.frame000.png", fixture.stem))
}

fn render_imqraw_artifact(options: &Options, fixture: &RenderFixture, renderer: &str) -> PathBuf {
    options
        .render_parity_dir
        .join(renderer)
        .join(format!("{}.frame000.imqraw", fixture.stem))
}

fn render_report(options: &Options, fixture: &RenderFixture, renderer: &str) -> PathBuf {
    options.render_parity_dir.join("reports").join(format!(
        "{}.{renderer}-vs-three-vrm.psnr.json",
        fixture.stem
    ))
}

fn render_imqraw_report(options: &Options, fixture: &RenderFixture, renderer: &str) -> PathBuf {
    options.render_parity_dir.join("reports").join(format!(
        "{}.{renderer}-vs-three-vrm.imqraw-rust.json",
        fixture.stem
    ))
}

fn render_review_manifest_path(options: &Options) -> PathBuf {
    options.render_parity_dir.join("review-manifest.json")
}

fn render_diff_png(options: &Options, fixture: &RenderFixture, renderer: &str) -> PathBuf {
    options
        .render_parity_dir
        .join("diff")
        .join(format!("{}.{renderer}-vs-three-vrm.diff.png", fixture.stem))
}

fn write_render_png_from_artifact(
    options: &Options,
    fixture: &RenderFixture,
    renderer: &str,
) -> Result<(), String> {
    let artifact = read_rgba_artifact(&render_artifact(options, fixture, renderer))?;
    let out = render_png(options, fixture, renderer);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", path(parent)))?;
    }
    image::save_buffer(
        &out,
        &artifact.rgba,
        artifact.width,
        artifact.height,
        image::ColorType::Rgba8,
    )
    .map_err(|err| format!("failed to write {}: {err}", path(&out)))?;
    verify_render_png_matches_artifact(&artifact, &out, fixture, renderer)
}

fn verify_render_png_matches_artifact(
    artifact: &RgbaArtifact,
    png_path: &Path,
    fixture: &RenderFixture,
    renderer: &str,
) -> Result<(), String> {
    let image = image::ImageReader::open(png_path)
        .map_err(|err| format!("failed to open {}: {err}", path(png_path)))?
        .with_guessed_format()
        .map_err(|err| format!("failed to guess image format for {}: {err}", path(png_path)))?
        .decode()
        .map_err(|err| format!("failed to decode {}: {err}", path(png_path)))?
        .to_rgba8();

    if image.width() != artifact.width || image.height() != artifact.height {
        return Err(format!(
            "{} {renderer} PNG dimensions differ from RGBA artifact: png {}x{}, artifact {}x{}",
            fixture.name,
            image.width(),
            image.height(),
            artifact.width,
            artifact.height
        ));
    }

    let png_rgba = image.as_raw();
    if png_rgba.as_slice() != artifact.rgba.as_slice() {
        let mismatches = png_rgba
            .chunks_exact(4)
            .zip(artifact.rgba.chunks_exact(4))
            .filter(|(png, artifact)| png[3] != artifact[3])
            .count();
        return Err(format!(
            "{} {renderer} PNG bytes differ from RGBA artifact; alpha mismatches={mismatches}",
            fixture.name
        ));
    }

    let stats = alpha_stats(artifact);
    println!(
        "png-alpha {} {renderer}: transparent={} opaque={} partial={}",
        fixture.name, stats.transparent, stats.opaque, stats.partial
    );
    Ok(())
}

fn write_render_diff_image(
    options: &Options,
    fixture: &RenderFixture,
    renderer: &str,
) -> Result<(), String> {
    let expected = read_rgba_artifact(&render_artifact(options, fixture, "three-vrm"))?;
    let actual = read_rgba_artifact(&render_artifact(options, fixture, renderer))?;
    let diff = rgba_diff_heatmap(&expected, &actual, renderer)?;

    let out = render_diff_png(options, fixture, renderer);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", path(parent)))?;
    }
    image::save_buffer(
        &out,
        &diff.rgba,
        diff.width,
        diff.height,
        image::ColorType::Rgba8,
    )
    .map_err(|err| format!("failed to write {}: {err}", path(&out)))
}

fn rgba_diff_heatmap(
    expected: &RgbaArtifact,
    actual: &RgbaArtifact,
    label: &str,
) -> Result<RgbaArtifact, String> {
    if expected.width != actual.width || expected.height != actual.height {
        return Err(format!(
            "{label}: dimension mismatch: expected {}x{}, actual {}x{}",
            expected.width, expected.height, actual.width, actual.height
        ));
    }

    let rgba = expected
        .rgba
        .chunks_exact(4)
        .zip(actual.rgba.chunks_exact(4))
        .flat_map(|(expected, actual)| {
            let rgb_delta = expected[..3]
                .iter()
                .zip(&actual[..3])
                .map(|(left, right)| left.abs_diff(*right))
                .max()
                .unwrap_or(0);
            let alpha_delta = expected[3].abs_diff(actual[3]);
            let red = amplify_delta(rgb_delta);
            let blue = amplify_delta(alpha_delta);
            [red, 0, blue, 255]
        })
        .collect::<Vec<_>>();

    Ok(RgbaArtifact {
        width: expected.width,
        height: expected.height,
        rgba,
    })
}

fn amplify_delta(delta: u8) -> u8 {
    delta.saturating_mul(4)
}

fn verify_render_alpha_consistency(
    options: &Options,
    fixture: &RenderFixture,
) -> Result<(), String> {
    let reference = read_rgba_artifact(&render_artifact(options, fixture, "three-vrm"))?;
    let reference_stats = alpha_stats(&reference);
    match options.render_background {
        RenderBackground::Transparent if reference_stats.transparent == 0 => {
            return Err(format!(
                "{} three-vrm reference has no transparent background pixels; expected transparent RGBA capture",
                fixture.name
            ));
        }
        RenderBackground::OpaqueBlack
            if reference_stats.transparent != 0 || reference_stats.partial != 0 =>
        {
            return Err(format!(
                "{} three-vrm reference has transparent or partial-alpha pixels under opaque-black background: transparent={} partial={}",
                fixture.name, reference_stats.transparent, reference_stats.partial
            ));
        }
        _ => {}
    }

    for renderer in ["wgpu", "bevy"] {
        let actual = read_rgba_artifact(&render_artifact(options, fixture, renderer))?;
        if reference.width != actual.width || reference.height != actual.height {
            return Err(format!(
                "{renderer}: dimension mismatch: expected {}x{}, actual {}x{}",
                reference.width, reference.height, actual.width, actual.height
            ));
        }

        let stats = alpha_stats(&actual);
        match options.render_background {
            RenderBackground::Transparent if stats.transparent == 0 => {
                return Err(format!(
                    "{} {renderer} capture has no transparent background pixels; expected transparent RGBA capture",
                    fixture.name
                ));
            }
            RenderBackground::OpaqueBlack if stats.transparent != 0 || stats.partial != 0 => {
                return Err(format!(
                    "{} {renderer} capture has transparent or partial-alpha pixels under opaque-black background: transparent={} partial={}",
                    fixture.name, stats.transparent, stats.partial
                ));
            }
            _ => {}
        }

        let mismatches =
            alpha_mismatch_count(&reference, &actual, options.render_alpha_channel_tolerance);
        if mismatches > options.render_alpha_mismatch_tolerance {
            return Err(format!(
                "{} {renderer} alpha mask differs from three-vrm by {mismatches} pixels (pixel tolerance {}, channel tolerance {})",
                fixture.name,
                options.render_alpha_mismatch_tolerance,
                options.render_alpha_channel_tolerance
            ));
        }

        println!(
            "alpha {} {renderer}: transparent={} opaque={} partial={} mismatches={} channel_tolerance={}",
            fixture.name,
            stats.transparent,
            stats.opaque,
            stats.partial,
            mismatches,
            options.render_alpha_channel_tolerance
        );
    }

    println!(
        "alpha {} three-vrm: transparent={} opaque={} partial={}",
        fixture.name, reference_stats.transparent, reference_stats.opaque, reference_stats.partial
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AlphaStats {
    transparent: usize,
    opaque: usize,
    partial: usize,
}

fn alpha_stats(artifact: &RgbaArtifact) -> AlphaStats {
    artifact.rgba.chunks_exact(4).fold(
        AlphaStats {
            transparent: 0,
            opaque: 0,
            partial: 0,
        },
        |mut stats, pixel| {
            match pixel[3] {
                0 => stats.transparent += 1,
                255 => stats.opaque += 1,
                _ => stats.partial += 1,
            }
            stats
        },
    )
}

fn alpha_mismatch_count(
    expected: &RgbaArtifact,
    actual: &RgbaArtifact,
    channel_tolerance: u8,
) -> usize {
    expected
        .rgba
        .chunks_exact(4)
        .zip(actual.rgba.chunks_exact(4))
        .filter(|(expected, actual)| expected[3].abs_diff(actual[3]) > channel_tolerance)
        .count()
}

#[derive(Debug)]
struct RgbaArtifact {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn read_rgba_artifact(path: &Path) -> Result<RgbaArtifact, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", self::path(path)))?;
    parse_rgba_artifact_json(&text, &self::path(path))
}

fn parse_rgba_artifact_json(text: &str, source: &str) -> Result<RgbaArtifact, String> {
    let value = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|err| format!("failed to parse {source}: {err}"))?;
    parse_rgba_artifact_value(&value, source)
}

fn parse_rgba_artifact_value(
    value: &serde_json::Value,
    source: &str,
) -> Result<RgbaArtifact, String> {
    let width = json_u32(value, "width", source)?;
    let height = json_u32(value, "height", source)?;
    let rgba_values = value
        .get("rgba")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("{source}: rgba must be an array"))?;
    let expected_len = width as usize * height as usize * 4;
    if rgba_values.len() != expected_len {
        return Err(format!(
            "{source}: rgba length {} does not match {expected_len}",
            rgba_values.len()
        ));
    }
    let rgba = rgba_values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let value = value
                .as_u64()
                .ok_or_else(|| format!("{source}: rgba[{index}] must be an integer"))?;
            u8::try_from(value).map_err(|_| format!("{source}: rgba[{index}] must be in 0..255"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RgbaArtifact {
        width,
        height,
        rgba,
    })
}

fn json_u32(value: &serde_json::Value, field: &str, source: &str) -> Result<u32, String> {
    let value = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("{source}: {field} must be a positive integer"))?;
    let value =
        u32::try_from(value).map_err(|_| format!("{source}: {field} is too large for u32"))?;
    if value == 0 {
        Err(format!("{source}: {field} must be a positive integer"))
    } else {
        Ok(value)
    }
}

fn render_summary_markdown(
    options: &Options,
    fixtures: &[RenderFixture],
) -> Result<String, String> {
    let meta = RenderSummaryMeta {
        artifacts: path(&options.render_parity_dir),
        visual_review: path(&options.render_parity_dir.join("visual-review.html")),
        numeric_gate: "direct .imqraw via tools/render-parity/compare-imqraw.rs".to_owned(),
        metric: options.render_psnr_metric.as_cli_value().to_owned(),
        background: options.render_background.as_cli_value().to_owned(),
        mtoon_light_accumulation: options
            .render_mtoon_light_accumulation
            .as_cli_value()
            .to_owned(),
        diagnostic_mode: options.render_diagnostic_mode.as_cli_value().to_owned(),
        normal_map_mode: options.render_normal_map_mode.as_cli_value().to_owned(),
        normal_map_scale: options.render_normal_map_scale,
        disable_texture_mips: options.render_disable_texture_mips,
        alpha_mismatch_tolerance: options.render_alpha_mismatch_tolerance,
        alpha_channel_tolerance: options.render_alpha_channel_tolerance,
    };
    let rows = fixtures
        .iter()
        .flat_map(|fixture| ["wgpu", "bevy"].map(move |renderer| (fixture, renderer)))
        .map(|(fixture, renderer)| {
            Ok(RenderSummaryRow {
                fixture_name: fixture.name.clone(),
                renderer: renderer.to_owned(),
                report: render_report_summary(options, fixture, renderer)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(render_summary_markdown_from_rows(&meta, &rows))
}

fn write_render_summary(options: &Options, summary: &str) -> Result<(), String> {
    let out = options.render_parity_dir.join("summary.md");
    std::fs::write(&out, summary).map_err(|err| format!("failed to write {}: {err}", path(&out)))
}

fn write_render_review_manifest(
    options: &Options,
    fixtures: &[RenderFixture],
) -> Result<(), String> {
    let manifest = render_review_manifest_value(options, fixtures)?;
    let out = render_review_manifest_path(options);
    let text = serde_json::to_string_pretty(&manifest)
        .map_err(|err| format!("failed to serialize render review manifest: {err}"))?;
    std::fs::write(&out, format!("{text}\n"))
        .map_err(|err| format!("failed to write {}: {err}", path(&out)))
}

fn render_review_manifest_value(
    options: &Options,
    fixtures: &[RenderFixture],
) -> Result<serde_json::Value, String> {
    let fixtures = fixtures
        .iter()
        .map(|fixture| render_review_manifest_fixture(options, fixture))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::json!({
        "generatedBy": "cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity",
        "artifacts": path(&options.render_parity_dir),
        "summary": path(&options.render_parity_dir.join("summary.md")),
        "visualReview": path(&options.render_parity_dir.join("visual-review.html")),
        "numericGate": "direct .imqraw via tools/render-parity/compare-imqraw.rs",
        "metric": options.render_psnr_metric.as_cli_value(),
        "background": options.render_background.as_cli_value(),
        "mtoonLightAccumulation": options.render_mtoon_light_accumulation.as_cli_value(),
        "diagnosticMode": options.render_diagnostic_mode.as_cli_value(),
        "normalMapMode": options.render_normal_map_mode.as_cli_value(),
        "normalMapScale": options.render_normal_map_scale,
        "disableTextureMips": options.render_disable_texture_mips,
        "alphaMismatchTolerance": options.render_alpha_mismatch_tolerance,
        "alphaChannelTolerance": options.render_alpha_channel_tolerance,
        "fixtures": fixtures,
    }))
}

fn render_review_manifest_fixture(
    options: &Options,
    fixture: &RenderFixture,
) -> Result<serde_json::Value, String> {
    let comparisons = ["wgpu", "bevy"]
        .into_iter()
        .map(|renderer| render_review_manifest_comparison(options, fixture, renderer))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::json!({
        "name": &fixture.name,
        "stem": &fixture.stem,
        "source": path(&fixture.path),
        "reference": render_review_manifest_artifacts(options, fixture, "three-vrm"),
        "comparisons": comparisons,
    }))
}

fn render_review_manifest_comparison(
    options: &Options,
    fixture: &RenderFixture,
    renderer: &str,
) -> Result<serde_json::Value, String> {
    let report = render_report_summary(options, fixture, renderer)?;
    Ok(serde_json::json!({
        "renderer": renderer,
        "capture": render_review_manifest_artifacts(options, fixture, renderer),
        "numericReport": path(&render_imqraw_report(options, fixture, renderer)),
        "diagnosticReport": path(&render_report(options, fixture, renderer)),
        "diffPng": path(&render_diff_png(options, fixture, renderer)),
        "summary": {
            "selectedPsnr": report.selected_psnr,
            "maxChannelDelta": report.max_channel_delta,
            "alphaMismatches": report.alpha_mismatches,
            "alphaMaxDelta": report.alpha_max_delta,
            "pass": report.pass,
        },
    }))
}

fn render_review_manifest_artifacts(
    options: &Options,
    fixture: &RenderFixture,
    renderer: &str,
) -> serde_json::Value {
    serde_json::json!({
        "rgbaJson": path(&render_artifact(options, fixture, renderer)),
        "imqraw": path(&render_imqraw_artifact(options, fixture, renderer)),
        "png": path(&render_png(options, fixture, renderer)),
    })
}

#[derive(Clone, Debug)]
struct RenderReportSummary {
    selected_psnr: String,
    max_channel_delta: String,
    alpha_mismatches: String,
    alpha_max_delta: String,
    pass: bool,
}

#[derive(Clone, Debug)]
struct RenderSummaryMeta {
    artifacts: String,
    visual_review: String,
    numeric_gate: String,
    metric: String,
    background: String,
    mtoon_light_accumulation: String,
    diagnostic_mode: String,
    normal_map_mode: String,
    normal_map_scale: f32,
    disable_texture_mips: bool,
    alpha_mismatch_tolerance: usize,
    alpha_channel_tolerance: u8,
}

#[derive(Clone, Debug)]
struct RenderSummaryRow {
    fixture_name: String,
    renderer: String,
    report: RenderReportSummary,
}

fn render_summary_markdown_from_rows(
    meta: &RenderSummaryMeta,
    rows: &[RenderSummaryRow],
) -> String {
    let mut output = String::from("# vrm-rs Render Parity Summary\n\n");
    output.push_str(
        "Generated by `cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity`.\n\n",
    );
    output.push_str(&format!(
        "- Artifacts: `{}`\n- Visual review: `{}`\n- Numeric gate: `{}`\n- Metric: `{}`\n- Background: `{}`\n- MToon light accumulation: `{}`\n- Diagnostic mode: `{}`\n- Normal map mode: `{}`, scale `{:.4}`\n- Texture mips disabled: `{}`\n- Alpha mismatch tolerance: `{}` pixels, channel tolerance `{}`\n\n",
        meta.artifacts,
        meta.visual_review,
        meta.numeric_gate,
        meta.metric,
        meta.background,
        meta.mtoon_light_accumulation,
        meta.diagnostic_mode,
        meta.normal_map_mode,
        meta.normal_map_scale,
        meta.disable_texture_mips,
        meta.alpha_mismatch_tolerance,
        meta.alpha_channel_tolerance,
    ));
    output.push_str("| Fixture | Renderer | Selected PSNR | Max channel delta | Alpha mismatches | Alpha max delta | Pass |\n");
    output.push_str("| --- | --- | ---: | ---: | ---: | ---: | --- |\n");

    for row in rows {
        output.push_str(&format!(
            "| `{}` | `{}` | {} | {} | {} | {} | {} |\n",
            row.fixture_name,
            row.renderer,
            row.report.selected_psnr,
            row.report.max_channel_delta,
            row.report.alpha_mismatches,
            row.report.alpha_max_delta,
            if row.report.pass { "yes" } else { "no" },
        ));
    }
    output.push('\n');
    output
}

fn render_report_summary(
    options: &Options,
    fixture: &RenderFixture,
    renderer: &str,
) -> Result<RenderReportSummary, String> {
    let report = render_imqraw_report(options, fixture, renderer);
    let text = std::fs::read_to_string(&report)
        .map_err(|err| format!("failed to read {}: {err}", path(&report)))?;
    parse_render_report_summary_json(&text, &path(&report))
}

fn parse_render_report_summary_json(
    text: &str,
    source: &str,
) -> Result<RenderReportSummary, String> {
    let json = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|err| format!("failed to parse {source}: {err}"))?;
    render_report_summary_from_value(&json, source)
}

fn render_report_summary_from_value(
    json: &serde_json::Value,
    source: &str,
) -> Result<RenderReportSummary, String> {
    let selected = json
        .get("selectedMetric")
        .ok_or_else(|| format!("{source}: missing selectedMetric"))?;
    let alpha = json
        .get("alpha")
        .ok_or_else(|| format!("{source}: missing alpha"))?;
    Ok(RenderReportSummary {
        selected_psnr: json_f64_string(selected, "psnr", source)?,
        max_channel_delta: json_u64_string(selected, "maxChannelDelta", source)?,
        alpha_mismatches: json_u64_string(alpha, "mismatches", source)?,
        alpha_max_delta: json_u64_string(alpha, "maxDelta", source)?,
        pass: json_bool(json, "pass", source)?,
    })
}

fn json_f64_string(value: &serde_json::Value, field: &str, source: &str) -> Result<String, String> {
    let Some(value) = value.get(field) else {
        return Err(format!("{source}: {field} must be a number"));
    };
    if value.is_null() || value.as_str() == Some("Infinity") {
        return Ok("Infinity".to_owned());
    }
    let value = value
        .as_f64()
        .ok_or_else(|| format!("{source}: {field} must be a number"))?;
    Ok(format!("{value:.4}"))
}

fn json_u64_string(value: &serde_json::Value, field: &str, source: &str) -> Result<String, String> {
    let value = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("{source}: {field} must be an integer"))?;
    Ok(value.to_string())
}

fn json_bool(value: &serde_json::Value, field: &str, source: &str) -> Result<bool, String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| format!("{source}: {field} must be a boolean"))
}

fn write_render_visual_review(
    options: &Options,
    fixtures: &[RenderFixture],
    summary: &str,
) -> Result<(), String> {
    let sections = fixtures
        .iter()
        .map(|fixture| render_visual_review_section(options, fixture))
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>vrm-rs Render Parity</title>
  <style>
    :root {{ color-scheme: light dark; font-family: system-ui, sans-serif; }}
    body {{ margin: 24px; }}
    main {{ max-width: 1180px; margin: 0 auto; }}
    h1 {{ font-size: 22px; margin-bottom: 4px; }}
    h2 {{ margin-top: 28px; }}
    .meta {{ color: #666; margin-top: 0; }}
    .grid {{ display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; }}
    .diff-grid {{ display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px; margin-top: 20px; }}
    figure {{ margin: 0; border: 1px solid #9995; padding: 12px; }}
    img {{ width: 100%; image-rendering: pixelated; background: repeating-conic-gradient(#ddd 0 25%, #fff 0 50%) 0 / 24px 24px; }}
    figcaption {{ font-weight: 600; margin-top: 8px; }}
    pre {{ overflow-x: auto; border: 1px solid #9995; padding: 12px; }}
  </style>
</head>
<body>
<main>
  <h1>vrm-rs Render Parity</h1>
  <p class="meta">Generated by <code>cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity</code>.</p>
  <section>
    <h2>Summary</h2>
    <pre>{summary}</pre>
  </section>
  {sections}
</main>
</body>
</html>
"#,
        sections = sections,
        summary = html_escape(summary),
    );
    let out = options.render_parity_dir.join("visual-review.html");
    std::fs::write(&out, html).map_err(|err| format!("failed to write {}: {err}", path(&out)))
}

fn render_visual_review_section(
    options: &Options,
    fixture: &RenderFixture,
) -> Result<String, String> {
    Ok(format!(
        r#"<section>
  <h2>{fixture_name}</h2>
  <section class="grid">
    <figure>
      <img src="three-vrm/{stem}.frame000.png" alt="{fixture_name} three-vrm reference">
      <figcaption>three-vrm reference</figcaption>
    </figure>
    <figure>
      <img src="wgpu/{stem}.frame000.png" alt="{fixture_name} wgpu capture">
      <figcaption>wgpu capture</figcaption>
    </figure>
    <figure>
      <img src="bevy/{stem}.frame000.png" alt="{fixture_name} Bevy capture">
      <figcaption>Bevy capture</figcaption>
    </figure>
  </section>
  <section class="diff-grid">
    <figure>
      <img src="diff/{stem}.wgpu-vs-three-vrm.diff.png" alt="{fixture_name} wgpu diff heatmap">
      <figcaption>wgpu diff heatmap (red: RGB, blue: alpha)</figcaption>
    </figure>
    <figure>
      <img src="diff/{stem}.bevy-vs-three-vrm.diff.png" alt="{fixture_name} Bevy diff heatmap">
      <figcaption>Bevy diff heatmap (red: RGB, blue: alpha)</figcaption>
    </figure>
  </section>
  <h3>wgpu vs three-vrm</h3>
  <pre>{wgpu_report}</pre>
  <h3>Bevy vs three-vrm</h3>
  <pre>{bevy_report}</pre>
</section>"#,
        fixture_name = html_escape(&fixture.name),
        stem = html_escape(&fixture.stem),
        wgpu_report = html_escape(&report_text(options, fixture, "wgpu")?),
        bevy_report = html_escape(&report_text(options, fixture, "bevy")?),
    ))
}

fn report_text(
    options: &Options,
    fixture: &RenderFixture,
    renderer: &str,
) -> Result<String, String> {
    let imqraw_report = render_imqraw_report(options, fixture, renderer);
    let rgba_report = render_report(options, fixture, renderer);
    let imqraw_text = std::fs::read_to_string(&imqraw_report)
        .map_err(|err| format!("failed to read {}: {err}", path(&imqraw_report)))?;
    let rgba_text = std::fs::read_to_string(&rgba_report)
        .map_err(|err| format!("failed to read {}: {err}", path(&rgba_report)))?;
    Ok(format!(
        "Direct imqraw gate report ({})\n{}\nRGBA JSON diagnostic report ({})\n{}",
        path(&imqraw_report),
        imqraw_text,
        path(&rgba_report),
        rgba_text,
    ))
}

fn html_escape(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(character),
        }
    }
    output
}

fn run_cargo_test_with_env<const N: usize, const M: usize>(
    envs: [(&str, String); N],
    args: [&str; M],
) -> Result<(), String> {
    let mut command = Command::new("cargo");
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    run_command(command)
}

fn run_cmd<const N: usize>(program: &str, args: [&str; N]) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args);
    run_command(command)
}

fn run_cmd_in<const N: usize>(
    current_dir: &PathBuf,
    program: &str,
    args: [&str; N],
) -> Result<(), String> {
    let mut command = Command::new(program);
    command.current_dir(current_dir).args(args);
    run_command(command)
}

fn run_command(mut command: Command) -> Result<(), String> {
    configure_cargo_debug_info(&mut command);
    println!("> {}", display_command(&command));
    let status = command
        .status()
        .map_err(|err| format!("failed to spawn command: {err}"))?;
    ensure_success(status)
}

fn configure_cargo_debug_info(command: &mut Command) {
    if command.get_program() == OsStr::new("cargo") {
        command.env("CARGO_PROFILE_DEV_DEBUG", "1");
        command.env("CARGO_PROFILE_TEST_DEBUG", "1");
    }
}

fn ensure_success(status: ExitStatus) -> Result<(), String> {
    if status.success() {
        Ok(())
    } else {
        Err(format!("command exited with {status}"))
    }
}

fn display_command(command: &Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(shellish)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shellish(value: &OsStr) -> String {
    let text = value.to_string_lossy();
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text.into_owned()
    }
}

fn path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
