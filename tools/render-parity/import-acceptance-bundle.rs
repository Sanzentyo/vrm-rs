#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
---

//! Import one returned strict acceptance bundle into the returned-bundle root.

use clap::Parser;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "import-acceptance-bundle",
    about = "Copy a returned strict acceptance bundle into .external-fixtures/render-parity-acceptance-returned/<label>/acceptance-bundle"
)]
struct Options {
    #[arg(long)]
    bundle: Option<PathBuf>,
    #[arg(long)]
    label: Option<String>,
    #[arg(long, default_value = ".external-fixtures/render-parity-acceptance-returned")]
    returned_root: PathBuf,
    #[arg(long, default_value = ".external-fixtures/render-parity-acceptance-environments")]
    out_root: PathBuf,
    #[arg(long)]
    replace: bool,
    #[arg(long)]
    skip_validation: bool,
    #[arg(long)]
    apply: bool,
    #[arg(long, hide = true)]
    self_test: bool,
}

#[derive(Clone, Debug)]
struct ImportPlan {
    source_bundle: PathBuf,
    destination_bundle: PathBuf,
    smoke_json: PathBuf,
    smoke_markdown: PathBuf,
    files: Vec<(PathBuf, PathBuf)>,
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
    let plan = build_import_plan(&options)?;
    if !options.skip_validation {
        validate_bundle(&plan.source_bundle)?;
    }
    if !options.apply {
        print_dry_run(&plan);
        return Ok(());
    }
    write_import(&plan, options.replace)?;
    if !options.skip_validation {
        validate_bundle_root(&options.returned_root, &plan.smoke_json, &plan.smoke_markdown)?;
    }
    println!(
        "imported acceptance bundle {} -> {}",
        display_path(&plan.source_bundle),
        display_path(&plan.destination_bundle)
    );
    Ok(())
}

fn build_import_plan(options: &Options) -> Result<ImportPlan, Box<dyn Error>> {
    let label_value = options
        .label
        .as_deref()
        .ok_or("--label is required unless --self-test is used")?;
    let label = validate_label(label_value)?;
    let source_bundle = options
        .bundle
        .clone()
        .ok_or("--bundle is required unless --self-test is used")?;
    if !source_bundle.join("bundle-manifest.json").is_file() {
        return Err(format!(
            "{} is not an acceptance bundle directory with bundle-manifest.json",
            display_path(&source_bundle)
        )
        .into());
    }
    let destination_bundle = options
        .returned_root
        .join(label)
        .join("acceptance-bundle");
    let smoke_json = options
        .out_root
        .join(format!("import-{label}-strict-smoke.json"));
    let smoke_markdown = options
        .out_root
        .join(format!("import-{label}-strict-smoke.md"));
    let files = copy_pairs(&source_bundle, &destination_bundle)?;
    if files.is_empty() {
        return Err(format!("{} has no files to import", display_path(&source_bundle)).into());
    }
    Ok(ImportPlan {
        source_bundle,
        destination_bundle,
        smoke_json,
        smoke_markdown,
        files,
    })
}

fn validate_label(label: &str) -> Result<&str, Box<dyn Error>> {
    let label = label.trim();
    if label.is_empty() {
        return Err("--label must not be empty".into());
    }
    if label == "." || label == ".." {
        return Err("--label must not be . or ..".into());
    }
    if !label
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err("--label may contain only ASCII letters, digits, '-' and '_'".into());
    }
    Ok(label)
}

fn copy_pairs(source: &Path, destination: &Path) -> Result<Vec<(PathBuf, PathBuf)>, Box<dyn Error>> {
    let mut pairs = Vec::new();
    let mut stack = vec![source.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let relative = path.strip_prefix(source)?;
                pairs.push((path.clone(), destination.join(relative)));
            }
        }
    }
    pairs.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(pairs)
}

fn write_import(plan: &ImportPlan, replace: bool) -> Result<(), Box<dyn Error>> {
    if plan.destination_bundle.exists() {
        if !replace {
            return Err(format!(
                "{} already exists; pass --replace to overwrite it",
                display_path(&plan.destination_bundle)
            )
            .into());
        }
        fs::remove_dir_all(&plan.destination_bundle)?;
    }
    for (source, destination) in &plan.files {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn validate_bundle(bundle: &Path) -> Result<(), Box<dyn Error>> {
    run_command(
        Command::new("cargo")
            .args([
                "+nightly",
                "-Zscript",
                "tools/render-parity/validate-acceptance-environments.rs",
                "--bundle",
            ])
            .arg(bundle)
            .args(["--min-environments", "1", "--require-accepted-signoff"]),
    )
}

fn validate_bundle_root(
    returned_root: &Path,
    json_out: &Path,
    markdown_out: &Path,
) -> Result<(), Box<dyn Error>> {
    run_command(
        Command::new("cargo")
            .args([
                "+nightly",
                "-Zscript",
                "tools/render-parity/validate-acceptance-environments.rs",
                "--bundle-root",
            ])
            .arg(returned_root)
            .args(["--min-environments", "1", "--require-accepted-signoff", "--json-out"])
            .arg(json_out)
            .arg("--markdown-out")
            .arg(markdown_out),
    )
}

fn run_command(command: &mut Command) -> Result<(), Box<dyn Error>> {
    println!("running: {}", display_command(command));
    let status = command.status()?;
    if !status.success() {
        return Err(format!(
            "command failed with {}: {}",
            display_status(status),
            display_command(command)
        )
        .into());
    }
    Ok(())
}

fn print_dry_run(plan: &ImportPlan) {
    println!(
        "dry run: would import {} files from {} to {}",
        plan.files.len(),
        display_path(&plan.source_bundle),
        display_path(&plan.destination_bundle)
    );
    println!(
        "would write smoke reports: {}, {}",
        display_path(&plan.smoke_json),
        display_path(&plan.smoke_markdown)
    );
    println!("rerun with --apply to copy the bundle");
}

fn display_command(command: &Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|part| shell_quote(&part.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(text: &str) -> String {
    if text
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | '\\' | ':'))
    {
        return text.to_owned();
    }
    format!("\"{}\"", text.replace('"', "\\\""))
}

fn display_status(status: ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "terminated by signal".to_owned())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from("target/acceptance-bundle-import-self-test");
    let source = root.join("source-bundle");
    let returned = root.join("returned");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(source.join("run-1"))?;
    fs::write(source.join("bundle-manifest.json"), "{}\n")?;
    fs::write(source.join("acceptance-repeat-summary.json"), "{}\n")?;
    fs::write(source.join("run-1").join("summary.md"), "# run\n")?;

    let options = Options {
        bundle: Some(source.clone()),
        label: Some("gpu_a".to_owned()),
        returned_root: returned.clone(),
        out_root: root.join("out"),
        replace: false,
        skip_validation: true,
        apply: false,
        self_test: false,
    };
    let plan = build_import_plan(&options)?;
    if plan.files.len() != 3 {
        return Err(format!("expected 3 copied files, got {}", plan.files.len()).into());
    }
    if plan.destination_bundle != returned.join("gpu_a").join("acceptance-bundle") {
        return Err("destination layout is wrong".into());
    }
    for bad_label in ["", ".", "..", "../x", "gpu/a", "gpu a"] {
        let mut bad = options.clone();
        bad.label = Some(bad_label.to_owned());
        if build_import_plan(&bad).is_ok() {
            return Err(format!("bad label {bad_label:?} should be rejected").into());
        }
    }
    write_import(&plan, false)?;
    if !plan
        .destination_bundle
        .join("acceptance-repeat-summary.json")
        .is_file()
    {
        return Err("summary was not copied".into());
    }
    if write_import(&plan, false).is_ok() {
        return Err("existing destination should require --replace".into());
    }
    write_import(&plan, true)?;
    Ok(())
}
