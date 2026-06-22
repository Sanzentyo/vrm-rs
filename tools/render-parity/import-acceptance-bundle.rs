#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
zip = { version = "9.0.0-pre2", default-features = false, features = ["deflate"] }
---

//! Import one returned strict acceptance bundle into the returned-bundle root.

use clap::Parser;
use std::error::Error;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use zip::write::SimpleFileOptions;
use zip::ZipArchive;
use zip::ZipWriter;

const ZIP_IMPORT_WORK_ROOT: &str = "target/acceptance-bundle-import-zip";

#[derive(Clone, Debug, Parser)]
#[command(
    name = "import-acceptance-bundle",
    about = "Copy a returned strict acceptance bundle directory or .zip into .external-fixtures/render-parity-acceptance-returned/<label>/acceptance-bundle"
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

#[derive(Clone, Debug)]
struct ImportRequest {
    source: PathBuf,
    label: String,
    destination_bundle: PathBuf,
    smoke_json: PathBuf,
    smoke_markdown: PathBuf,
}

#[derive(Clone, Debug)]
struct ZipPreview {
    bundle_root: PathBuf,
    file_count: usize,
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
    let request = build_import_request(&options)?;
    if request.source.is_file() && is_zip_path(&request.source) && !options.apply {
        let preview = preview_zip_bundle(&request.source)?;
        print_zip_dry_run(&request, &preview);
        return Ok(());
    }
    let source_bundle = prepare_source_bundle(&request)?;
    let plan = build_import_plan(&request, &source_bundle)?;
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

fn build_import_request(options: &Options) -> Result<ImportRequest, Box<dyn Error>> {
    let label_value = options
        .label
        .as_deref()
        .ok_or("--label is required unless --self-test is used")?;
    let label = validate_label(label_value)?.to_owned();
    let source = options
        .bundle
        .clone()
        .ok_or("--bundle is required unless --self-test is used")?;
    let destination_bundle = options
        .returned_root
        .join(&label)
        .join("acceptance-bundle");
    let smoke_json = options
        .out_root
        .join(format!("import-{label}-strict-smoke.json"));
    let smoke_markdown = options
        .out_root
        .join(format!("import-{label}-strict-smoke.md"));
    Ok(ImportRequest {
        source,
        label,
        destination_bundle,
        smoke_json,
        smoke_markdown,
    })
}

fn build_import_plan(
    request: &ImportRequest,
    source_bundle: &Path,
) -> Result<ImportPlan, Box<dyn Error>> {
    if !source_bundle.join("bundle-manifest.json").is_file() {
        return Err(format!(
            "{} is not an acceptance bundle directory with bundle-manifest.json",
            display_path(source_bundle)
        )
        .into());
    }
    let files = copy_pairs(source_bundle, &request.destination_bundle)?;
    if files.is_empty() {
        return Err(format!("{} has no files to import", display_path(source_bundle)).into());
    }
    Ok(ImportPlan {
        source_bundle: source_bundle.to_path_buf(),
        destination_bundle: request.destination_bundle.clone(),
        smoke_json: request.smoke_json.clone(),
        smoke_markdown: request.smoke_markdown.clone(),
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

fn prepare_source_bundle(request: &ImportRequest) -> Result<PathBuf, Box<dyn Error>> {
    if request.source.is_dir() {
        return Ok(request.source.clone());
    }
    if request.source.is_file() && is_zip_path(&request.source) {
        return extract_zip_bundle(&request.source, &request.label);
    }
    Err(format!(
        "{} is neither an acceptance bundle directory nor a .zip file",
        display_path(&request.source)
    )
    .into())
}

fn is_zip_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

fn preview_zip_bundle(path: &Path) -> Result<ZipPreview, Box<dyn Error>> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    let bundle_root = find_zip_bundle_root(&mut archive)?;
    let file_count = count_zip_bundle_files(&mut archive, &bundle_root)?;
    Ok(ZipPreview {
        bundle_root,
        file_count,
    })
}

fn extract_zip_bundle(path: &Path, label: &str) -> Result<PathBuf, Box<dyn Error>> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    let bundle_root = find_zip_bundle_root(&mut archive)?;
    let extract_root = PathBuf::from(ZIP_IMPORT_WORK_ROOT)
        .join(label)
        .join("acceptance-bundle");
    if extract_root.exists() {
        fs::remove_dir_all(&extract_root)?;
    }
    fs::create_dir_all(&extract_root)?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let Some(path) = file.enclosed_name() else {
            return Err(format!("zip entry {:?} is not enclosed", file.name()).into());
        };
        if !path_is_under_bundle_root(&path, &bundle_root) {
            continue;
        }
        let relative = strip_bundle_root(&path, &bundle_root)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let destination = extract_root.join(relative);
        if file.is_dir() {
            fs::create_dir_all(&destination)?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&destination)?;
        io::copy(&mut file, &mut output)?;
    }
    Ok(extract_root)
}

fn find_zip_bundle_root<R: io::Read + io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<PathBuf, Box<dyn Error>> {
    let mut candidates = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let Some(path) = file.enclosed_name() else {
            return Err(format!("zip entry {:?} is not enclosed", file.name()).into());
        };
        if path.file_name().and_then(|name| name.to_str()) == Some("bundle-manifest.json") {
            candidates.push(path.parent().unwrap_or_else(|| Path::new("")).to_path_buf());
        }
    }
    candidates.sort();
    candidates.dedup();
    match candidates.as_slice() {
        [bundle_root] => Ok(bundle_root.clone()),
        [] => Err("zip does not contain bundle-manifest.json".into()),
        _ => Err(format!(
            "zip contains multiple bundle-manifest.json roots: {}",
            candidates
                .iter()
                .map(|path| display_path(path))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into()),
    }
}

fn count_zip_bundle_files<R: io::Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    bundle_root: &Path,
) -> Result<usize, Box<dyn Error>> {
    let mut count = 0;
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let Some(path) = file.enclosed_name() else {
            return Err(format!("zip entry {:?} is not enclosed", file.name()).into());
        };
        if !file.is_dir() && path_is_under_bundle_root(&path, bundle_root) {
            count += 1;
        }
    }
    Ok(count)
}

fn path_is_under_bundle_root(path: &Path, bundle_root: &Path) -> bool {
    bundle_root.as_os_str().is_empty() || path.starts_with(bundle_root)
}

fn strip_bundle_root(path: &Path, bundle_root: &Path) -> Result<PathBuf, Box<dyn Error>> {
    if bundle_root.as_os_str().is_empty() {
        return Ok(path.to_path_buf());
    }
    Ok(path.strip_prefix(bundle_root)?.to_path_buf())
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

fn print_zip_dry_run(request: &ImportRequest, preview: &ZipPreview) {
    println!(
        "dry run: would extract {} files from zip {} root {}",
        preview.file_count,
        display_path(&request.source),
        display_path(&preview.bundle_root)
    );
    println!(
        "then import extracted bundle to {}",
        display_path(&request.destination_bundle)
    );
    println!(
        "would write smoke reports: {}, {}",
        display_path(&request.smoke_json),
        display_path(&request.smoke_markdown)
    );
    println!("rerun with --apply to extract, validate, and copy the bundle");
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
    let request = build_import_request(&options)?;
    let plan = build_import_plan(&request, &source)?;
    if plan.files.len() != 3 {
        return Err(format!("expected 3 copied files, got {}", plan.files.len()).into());
    }
    if plan.destination_bundle != returned.join("gpu_a").join("acceptance-bundle") {
        return Err("destination layout is wrong".into());
    }
    for bad_label in ["", ".", "..", "../x", "gpu/a", "gpu a"] {
        let mut bad = options.clone();
        bad.label = Some(bad_label.to_owned());
        if build_import_request(&bad).is_ok() {
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
    let zip_path = root.join("source-bundle.zip");
    write_test_zip(&zip_path)?;
    let zip_preview = preview_zip_bundle(&zip_path)?;
    if zip_preview.bundle_root != PathBuf::from("wrapped").join("acceptance-bundle") {
        return Err(format!(
            "zip bundle root was {}, expected wrapped/acceptance-bundle",
            display_path(&zip_preview.bundle_root)
        )
        .into());
    }
    if zip_preview.file_count != 3 {
        return Err(format!(
            "expected 3 zip bundle files, got {}",
            zip_preview.file_count
        )
        .into());
    }
    let zip_options = Options {
        bundle: Some(zip_path),
        label: Some("gpu_zip".to_owned()),
        returned_root: returned.clone(),
        out_root: root.join("out"),
        replace: false,
        skip_validation: true,
        apply: true,
        self_test: false,
    };
    let zip_request = build_import_request(&zip_options)?;
    let extracted = prepare_source_bundle(&zip_request)?;
    if !extracted.join("bundle-manifest.json").is_file()
        || !extracted.join("run-1").join("summary.md").is_file()
    {
        return Err("zip bundle was not extracted into an acceptance bundle root".into());
    }
    let zip_plan = build_import_plan(&zip_request, &extracted)?;
    write_import(&zip_plan, false)?;
    if !zip_plan
        .destination_bundle
        .join("acceptance-repeat-summary.json")
        .is_file()
    {
        return Err("zip-imported summary was not copied".into());
    }
    Ok(())
}

fn write_test_zip(path: &Path) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    let mut writer = ZipWriter::new(file);
    let options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, contents) in [
        (
            "wrapped/acceptance-bundle/bundle-manifest.json",
            "{}\n",
        ),
        (
            "wrapped/acceptance-bundle/acceptance-repeat-summary.json",
            "{}\n",
        ),
        ("wrapped/acceptance-bundle/run-1/summary.md", "# run\n"),
        ("unrelated/readme.txt", "ignored\n"),
    ] {
        writer.start_file(name, options)?;
        writer.write_all(contents.as_bytes())?;
    }
    writer.finish()?;
    Ok(())
}
