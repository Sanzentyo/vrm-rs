#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
zip = { version = "9.0.0-pre2", default-features = false, features = ["deflate"] }
---

//! Package a small transfer directory as a portable zip archive.

use clap::Parser;
use std::error::Error;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "package-transfer-zip",
    about = "Package a source-like transfer directory into a zip archive"
)]
struct Options {
    #[arg(long)]
    input_dir: Option<PathBuf>,
    #[arg(long)]
    zip_out: Option<PathBuf>,
    #[arg(long)]
    root_name: Option<String>,
    #[arg(long)]
    require_file: Vec<PathBuf>,
    #[arg(long)]
    apply: bool,
    #[arg(long, hide = true)]
    self_test: bool,
}

#[derive(Clone, Debug)]
struct ZipPlan {
    input_dir: PathBuf,
    zip_out: PathBuf,
    root_name: String,
    files: Vec<PathBuf>,
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
    let plan = build_plan(&options)?;
    if !options.apply {
        println!(
            "dry run: would write {} files from {} to {} with zip root {}",
            plan.files.len(),
            display_path(&plan.input_dir),
            display_path(&plan.zip_out),
            plan.root_name
        );
        println!("rerun with --apply to write the zip");
        return Ok(());
    }
    write_zip(&plan)?;
    println!(
        "wrote transfer zip: {} ({} files)",
        display_path(&plan.zip_out),
        plan.files.len()
    );
    Ok(())
}

fn build_plan(options: &Options) -> Result<ZipPlan, Box<dyn Error>> {
    let input_dir = options
        .input_dir
        .clone()
        .ok_or("--input-dir is required unless --self-test is used")?;
    if !input_dir.is_dir() {
        return Err(format!("{} is not a directory", display_path(&input_dir)).into());
    }
    let zip_out = options
        .zip_out
        .clone()
        .ok_or("--zip-out is required unless --self-test is used")?;
    let root_name = match options.root_name.as_deref() {
        Some(root_name) => validate_root_name(root_name)?.to_owned(),
        None => validate_root_name(
            input_dir
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("--root-name is required when --input-dir has no final component")?,
        )?
        .to_owned(),
    };
    validate_required_files(&input_dir, &options.require_file)?;
    validate_output_path(&input_dir, &zip_out)?;
    let files = collect_files(&input_dir)?;
    if files.is_empty() {
        return Err(format!("{} has no files to package", display_path(&input_dir)).into());
    }
    Ok(ZipPlan {
        input_dir,
        zip_out,
        root_name,
        files,
    })
}

fn validate_root_name(root_name: &str) -> Result<&str, Box<dyn Error>> {
    let root_name = root_name.trim();
    if root_name.is_empty() {
        return Err("--root-name must not be empty".into());
    }
    if root_name == "." || root_name == ".." {
        return Err("--root-name must not be . or ..".into());
    }
    if !root_name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err("--root-name may contain only ASCII letters, digits, '.', '-' and '_'".into());
    }
    Ok(root_name)
}

fn validate_required_files(input_dir: &Path, required: &[PathBuf]) -> Result<(), Box<dyn Error>> {
    for relative in required {
        validate_relative_path(relative)?;
        let path = input_dir.join(relative);
        if !path.is_file() {
            return Err(format!("required file is missing: {}", display_path(&path)).into());
        }
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<(), Box<dyn Error>> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "required file path must be relative and enclosed: {}",
            display_path(path)
        )
        .into());
    }
    Ok(())
}

fn validate_output_path(input_dir: &Path, zip_out: &Path) -> Result<(), Box<dyn Error>> {
    let input_abs = canonical_or_absolute(input_dir)?;
    let zip_abs = canonical_or_absolute(zip_out)?;
    if zip_abs == input_abs || zip_abs.starts_with(&input_abs) {
        return Err("refusing to write the zip inside the input directory".into());
    }
    Ok(())
}

fn collect_files(input_dir: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = Vec::new();
    let mut stack = vec![input_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let path = entry.path();
            if file_type.is_symlink() {
                return Err(format!("refusing to package symlink: {}", display_path(&path)).into());
            }
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn write_zip(plan: &ZipPlan) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = plan
        .zip_out
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(&plan.zip_out)?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for source in &plan.files {
        let relative = source.strip_prefix(&plan.input_dir)?;
        let entry_name = zip_entry_name(&plan.root_name, relative);
        writer.start_file(entry_name, options)?;
        writer.write_all(&fs::read(source)?)?;
    }
    writer.finish()?;
    Ok(())
}

fn zip_entry_name(root_name: &str, relative: &Path) -> String {
    let relative = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    format!("{root_name}/{relative}")
}

fn canonical_or_absolute(path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        if parent.exists() {
            return Ok(parent
                .canonicalize()?
                .join(path.file_name().unwrap_or_default()));
        }
    }
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    })
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn run_self_test() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from("target/package-transfer-zip-self-test");
    let input = root.join("runner-kit");
    let zip_out = root.join("runner-kit.zip");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(input.join("nested"))?;
    fs::write(input.join("README.md"), "# kit\n")?;
    fs::write(input.join("handoff.json"), "{}\n")?;
    fs::write(input.join("nested").join("RETURN.md"), "# return\n")?;
    let options = Options {
        input_dir: Some(input.clone()),
        zip_out: Some(zip_out.clone()),
        root_name: Some("render-parity-acceptance-runner-kit".to_owned()),
        require_file: vec![PathBuf::from("README.md"), PathBuf::from("handoff.json")],
        apply: true,
        self_test: false,
    };
    let plan = build_plan(&options)?;
    if plan.files.len() != 3 {
        return Err(format!("expected 3 files, got {}", plan.files.len()).into());
    }
    write_zip(&plan)?;
    let mut archive = ZipArchive::new(File::open(&zip_out)?)?;
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        entries.push(archive.by_index(index)?.name()?.into_owned());
    }
    entries.sort();
    for expected in [
        "render-parity-acceptance-runner-kit/README.md",
        "render-parity-acceptance-runner-kit/handoff.json",
        "render-parity-acceptance-runner-kit/nested/RETURN.md",
    ] {
        if !entries.iter().any(|entry| entry == expected) {
            return Err(format!("zip is missing entry {expected:?}").into());
        }
    }
    let missing = Options {
        require_file: vec![PathBuf::from("missing.txt")],
        ..options.clone()
    };
    if build_plan(&missing).is_ok() {
        return Err("missing required file should be rejected".into());
    }
    let bad_root = Options {
        root_name: Some("../bad".to_owned()),
        ..options.clone()
    };
    if build_plan(&bad_root).is_ok() {
        return Err("bad root name should be rejected".into());
    }
    let inside_output = Options {
        zip_out: Some(input.join("bad.zip")),
        ..options
    };
    if build_plan(&inside_output).is_ok() {
        return Err("zip inside input dir should be rejected".into());
    }
    Ok(())
}
