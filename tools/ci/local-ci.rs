#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"
---

//! Local replacement for the removed GitHub Actions workflow.
//!
//! Usage:
//! cargo +nightly -Zscript tools/ci/local-ci.rs
//! cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures
//! cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

const THREE_VRM_COMMIT: &str = "9d125586f6d7da094b0ac5f204cebf19586f2397";
const THREE_VRM_VIEWER_COMMIT: &str = "75ab65c9d4e488521d41bff7f5cfd1976a0b16e8";
const VRM_SPEC_COMMIT: &str = "3942748efbc803b258e288e0f6c993c6bb96cebf";

fn main() {
    if let Err(err) = Options::parse(env::args_os().skip(1)).and_then(run) {
        eprintln!("local-ci failed: {err}");
        std::process::exit(1);
    }
}

#[derive(Clone, Debug)]
struct Options {
    external_fixtures: bool,
    render_parity: bool,
    skip_core: bool,
    skip_coverage: bool,
    skip_download: bool,
    skip_three_vrm_build: bool,
    skip_golden_generation: bool,
    skip_playwright_install: bool,
    render_width: u32,
    render_height: u32,
    render_camera_z: f32,
    render_fail_under: Option<f32>,
    fixture_dir: PathBuf,
    golden_dir: PathBuf,
    three_vrm_root: PathBuf,
    render_parity_dir: PathBuf,
}

impl Options {
    fn parse(args: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let cwd = env::current_dir().map_err(|err| err.to_string())?;
        let mut options = Self {
            external_fixtures: false,
            render_parity: false,
            skip_core: false,
            skip_coverage: false,
            skip_download: false,
            skip_three_vrm_build: false,
            skip_golden_generation: false,
            skip_playwright_install: false,
            render_width: 256,
            render_height: 256,
            render_camera_z: 3.0,
            render_fail_under: None,
            fixture_dir: cwd.join(".external-fixtures").join("official"),
            golden_dir: cwd.join(".external-fixtures").join("golden"),
            three_vrm_root: cwd.join(".external-fixtures").join("three-vrm"),
            render_parity_dir: cwd.join(".external-fixtures").join("render-parity"),
        };

        let args = args.collect::<Vec<_>>();
        let mut index = 0;
        while index < args.len() {
            let arg = args[index].to_string_lossy();
            match arg.as_ref() {
                "--" => {}
                "--external-fixtures" => options.external_fixtures = true,
                "--render-parity" => options.render_parity = true,
                "--skip-core" => options.skip_core = true,
                "--skip-coverage" => options.skip_coverage = true,
                "--skip-download" => options.skip_download = true,
                "--skip-three-vrm-build" => options.skip_three_vrm_build = true,
                "--skip-golden-generation" => options.skip_golden_generation = true,
                "--skip-playwright-install" => options.skip_playwright_install = true,
                "--fixture-dir" => {
                    index += 1;
                    options.fixture_dir = required_path(&args, index, "--fixture-dir")?;
                }
                "--golden-dir" => {
                    index += 1;
                    options.golden_dir = required_path(&args, index, "--golden-dir")?;
                }
                "--three-vrm-root" => {
                    index += 1;
                    options.three_vrm_root = required_path(&args, index, "--three-vrm-root")?;
                }
                "--render-parity-dir" => {
                    index += 1;
                    options.render_parity_dir = required_path(&args, index, "--render-parity-dir")?;
                }
                "--render-width" => {
                    index += 1;
                    options.render_width = required_parse(&args, index, "--render-width")?;
                }
                "--render-height" => {
                    index += 1;
                    options.render_height = required_parse(&args, index, "--render-height")?;
                }
                "--render-camera-z" => {
                    index += 1;
                    options.render_camera_z = required_parse(&args, index, "--render-camera-z")?;
                }
                "--render-fail-under" => {
                    index += 1;
                    options.render_fail_under =
                        Some(required_parse(&args, index, "--render-fail-under")?);
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                _ => return Err(format!("unknown argument: {arg}")),
            }
            index += 1;
        }

        Ok(options)
    }
}

fn required_path(args: &[OsString], index: usize, flag: &str) -> Result<PathBuf, String> {
    args.get(index)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing value for {flag}"))
}

fn required_parse<T: std::str::FromStr>(
    args: &[OsString],
    index: usize,
    flag: &str,
) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    args.get(index)
        .ok_or_else(|| format!("missing value for {flag}"))?
        .to_string_lossy()
        .parse()
        .map_err(|err| format!("invalid value for {flag}: {err}"))
}

fn print_help() {
    println!(
        "\
Local vrm-rs CI runner

Usage:
  cargo +nightly -Zscript tools/ci/local-ci.rs
  cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures
  cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity

Options:
  --external-fixtures        Download external samples, build three-vrm, generate goldens, and run ignored parity tests
  --render-parity            Render Seed-san through three-vrm, wgpu, and Bevy, then write PSNR reports
  --skip-core                Skip fmt/test/clippy
  --skip-coverage            Skip cargo-llvm-cov
  --skip-download            Reuse existing external fixture files
  --skip-three-vrm-build     Reuse an existing built three-vrm checkout
  --skip-golden-generation   Reuse existing generated golden JSON files
  --skip-playwright-install  Reuse existing local Playwright installation
  --fixture-dir PATH         Override .external-fixtures/official
  --golden-dir PATH          Override .external-fixtures/golden
  --three-vrm-root PATH      Override .external-fixtures/three-vrm
  --render-parity-dir PATH   Override .external-fixtures/render-parity
  --render-width N           Render width, default 256
  --render-height N          Render height, default 256
  --render-camera-z N        Camera Z distance, default 3.0
  --render-fail-under N      Optional PSNR threshold for wgpu and Bevy reports
"
    );
}

fn run(options: Options) -> Result<(), String> {
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
    std::fs::create_dir_all(&options.render_parity_dir).map_err(|err| err.to_string())?;
    capture_three_vrm_reference(options)?;
    capture_wgpu(options)?;
    capture_bevy(options)?;
    compare_render_pair(options, "wgpu")?;
    compare_render_pair(options, "bevy")?;
    write_render_visual_review(options)
}

fn download_external_fixtures(options: &Options) -> Result<(), String> {
    let fixtures = [
        (
            format!("https://raw.githubusercontent.com/vrm-c/vrm-specification/{VRM_SPEC_COMMIT}/samples/Seed-san/vrm/Seed-san.vrm"),
            options.fixture_dir.join("Seed-san.vrm"),
        ),
        (
            format!("https://raw.githubusercontent.com/vrm-c/vrm-specification/{VRM_SPEC_COMMIT}/samples/VRM1_Constraint_Twist_Sample/vrm/VRM1_Constraint_Twist_Sample.vrm"),
            options.fixture_dir.join("VRM1_Constraint_Twist_Sample.vrm"),
        ),
        (
            format!("https://raw.githubusercontent.com/vrm-c/vrm-specification/{VRM_SPEC_COMMIT}/samples/VRMC_materials_mtoon_UV_Animation_Test/vrm/VRMC_materials_mtoon_UV_Animation_Test.vrm"),
            options.fixture_dir.join("VRMC_materials_mtoon_UV_Animation_Test.vrm"),
        ),
        (
            format!("https://raw.githubusercontent.com/vrm-c/vrm-specification/{VRM_SPEC_COMMIT}/samples/VRMC_vrm_expressions_isBinary_Overridden/vrm/VRMC_vrm_expressions_isBinary_Overridden.vrm"),
            options.fixture_dir.join("VRMC_vrm_expressions_isBinary_Overridden.vrm"),
        ),
        (
            format!("https://raw.githubusercontent.com/vrm-c/vrm-specification/{VRM_SPEC_COMMIT}/samples/VRMC_vrm_expressions_isBinary_Overrides/vrm/VRMC_vrm_expressions_isBinary_Overrides.vrm"),
            options.fixture_dir.join("VRMC_vrm_expressions_isBinary_Overrides.vrm"),
        ),
        (
            format!("https://raw.githubusercontent.com/pixiv/three-vrm/{THREE_VRM_COMMIT}/packages/three-vrm-animation/examples/models/test.vrma"),
            options.fixture_dir.join("test.vrma"),
        ),
        (
            format!("https://raw.githubusercontent.com/pixiv/three-vrm/{THREE_VRM_VIEWER_COMMIT}/packages/vrm-viewer/examples/models/idle_loop.vrma"),
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

fn capture_three_vrm_reference(options: &Options) -> Result<(), String> {
    run_cmd(
        "node",
        [
            "tools/render-parity/three-vrm-browser-capture.mjs",
            "--fixture",
            path(&options.fixture_dir.join("Seed-san.vrm")).as_str(),
            "--three-vrm-root",
            path(&options.three_vrm_root).as_str(),
            "--out",
            path(&render_artifact(options, "three-vrm")).as_str(),
            "--png-out",
            path(&render_png(options, "three-vrm")).as_str(),
            "--width",
            options.render_width.to_string().as_str(),
            "--height",
            options.render_height.to_string().as_str(),
            "--camera-z",
            options.render_camera_z.to_string().as_str(),
        ],
    )
}

fn capture_wgpu(options: &Options) -> Result<(), String> {
    run_cmd(
        "cargo",
        [
            "run",
            "--example",
            "wgpu_render_capture",
            "--",
            "--fixture",
            path(&options.fixture_dir.join("Seed-san.vrm")).as_str(),
            "--out",
            path(&render_artifact(options, "wgpu")).as_str(),
            "--png-out",
            path(&render_png(options, "wgpu")).as_str(),
            "--width",
            options.render_width.to_string().as_str(),
            "--height",
            options.render_height.to_string().as_str(),
            "--camera-z",
            options.render_camera_z.to_string().as_str(),
        ],
    )
}

fn capture_bevy(options: &Options) -> Result<(), String> {
    run_cmd(
        "cargo",
        [
            "run",
            "--example",
            "bevy_render_capture",
            "--",
            "--fixture",
            path(&options.fixture_dir.join("Seed-san.vrm")).as_str(),
            "--out",
            path(&render_artifact(options, "bevy")).as_str(),
            "--png-out",
            path(&render_png(options, "bevy")).as_str(),
            "--width",
            options.render_width.to_string().as_str(),
            "--height",
            options.render_height.to_string().as_str(),
            "--camera-z",
            options.render_camera_z.to_string().as_str(),
        ],
    )
}

fn compare_render_pair(options: &Options, renderer: &str) -> Result<(), String> {
    let mut command = Command::new("node");
    command.args([
        "tools/render-parity/compare-psnr.mjs",
        "--expected",
        path(&render_artifact(options, "three-vrm")).as_str(),
        "--actual",
        path(&render_artifact(options, renderer)).as_str(),
        "--out",
        path(&render_report(options, renderer)).as_str(),
    ]);
    if let Some(fail_under) = options.render_fail_under {
        command.args(["--fail-under", fail_under.to_string().as_str()]);
    }
    run_command(command)
}

fn render_artifact(options: &Options, renderer: &str) -> PathBuf {
    options
        .render_parity_dir
        .join(renderer)
        .join("Seed-san.frame000.rgba.json")
}

fn render_png(options: &Options, renderer: &str) -> PathBuf {
    options
        .render_parity_dir
        .join(renderer)
        .join("Seed-san.frame000.png")
}

fn render_report(options: &Options, renderer: &str) -> PathBuf {
    options
        .render_parity_dir
        .join("reports")
        .join(format!("Seed-san.{renderer}-vs-three-vrm.psnr.json"))
}

fn write_render_visual_review(options: &Options) -> Result<(), String> {
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>vrm-rs Seed-san Render Parity</title>
  <style>
    :root {{ color-scheme: light dark; font-family: system-ui, sans-serif; }}
    body {{ margin: 24px; }}
    main {{ max-width: 1180px; margin: 0 auto; }}
    h1 {{ font-size: 22px; margin-bottom: 4px; }}
    .meta {{ color: #666; margin-top: 0; }}
    .grid {{ display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; }}
    figure {{ margin: 0; border: 1px solid #9995; padding: 12px; }}
    img {{ width: 100%; image-rendering: pixelated; background: repeating-conic-gradient(#ddd 0 25%, #fff 0 50%) 0 / 24px 24px; }}
    figcaption {{ font-weight: 600; margin-top: 8px; }}
    pre {{ overflow-x: auto; border: 1px solid #9995; padding: 12px; }}
  </style>
</head>
<body>
<main>
  <h1>Seed-san Render Parity</h1>
  <p class="meta">Generated by <code>cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity</code>.</p>
  <section class="grid">
    <figure>
      <img src="three-vrm/Seed-san.frame000.png" alt="three-vrm reference">
      <figcaption>three-vrm reference</figcaption>
    </figure>
    <figure>
      <img src="wgpu/Seed-san.frame000.png" alt="wgpu capture">
      <figcaption>wgpu capture</figcaption>
    </figure>
    <figure>
      <img src="bevy/Seed-san.frame000.png" alt="Bevy capture">
      <figcaption>Bevy capture</figcaption>
    </figure>
  </section>
  <h2>wgpu vs three-vrm</h2>
  <pre>{wgpu_report}</pre>
  <h2>Bevy vs three-vrm</h2>
  <pre>{bevy_report}</pre>
</main>
</body>
</html>
"#,
        wgpu_report = html_escape(&report_text(options, "wgpu")?),
        bevy_report = html_escape(&report_text(options, "bevy")?),
    );
    let out = options.render_parity_dir.join("visual-review.html");
    std::fs::write(&out, html).map_err(|err| format!("failed to write {}: {err}", path(&out)))
}

fn report_text(options: &Options, renderer: &str) -> Result<String, String> {
    let report = render_report(options, renderer);
    std::fs::read_to_string(&report)
        .map_err(|err| format!("failed to read {}: {err}", path(&report)))
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
    println!("> {}", display_command(&command));
    let status = command
        .status()
        .map_err(|err| format!("failed to spawn command: {err}"))?;
    ensure_success(status)
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

fn path(path: &PathBuf) -> String {
    path.to_string_lossy().into_owned()
}
