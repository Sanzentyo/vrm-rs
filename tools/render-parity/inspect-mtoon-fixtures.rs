#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[package]
edition = "2024"

[dependencies]
clap = { version = "4.6.1", features = ["derive"] }
gltf = { version = "1.4.1", features = ["extensions", "KHR_texture_transform"] }
serde_json = "1.0.150"
---

use clap::{Parser, ValueEnum};
use serde_json::Value;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "inspect-mtoon-fixtures",
    about = "Inspect local VRM/glTF fixtures for MToon material parity features"
)]
struct Options {
    #[arg(long, default_value = ".external-fixtures/official")]
    root: PathBuf,
    #[arg(long, value_enum, default_value_t = OutputFormat::Markdown)]
    format: OutputFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Markdown,
    Json,
}

#[derive(Clone, Debug, Default)]
struct FixtureSummary {
    path: PathBuf,
    vrm0_materials: usize,
    vrm1_materials: usize,
    outline_none: usize,
    outline_world: usize,
    outline_screen: usize,
    outline_width_textures: usize,
    alpha_blend: usize,
    alpha_mask: usize,
    transparent_z_write: usize,
    shade_textures: usize,
    normal_textures: usize,
    matcap_textures: usize,
    rim_textures: usize,
    shading_shift_textures: usize,
    uv_animation: usize,
    uv_animation_mask_textures: usize,
    load_error: Option<String>,
}

impl FixtureSummary {
    fn mtoon_total(&self) -> usize {
        self.vrm0_materials + self.vrm1_materials
    }

    fn has_features(&self) -> bool {
        self.load_error.is_some() || self.mtoon_total() > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutlineMode {
    None,
    World,
    Screen,
}

fn main() -> Result<(), String> {
    let options = Options::parse_from(script_args());
    let summaries = supported_fixtures_under(&options.root)
        .into_iter()
        .map(inspect_fixture)
        .filter(FixtureSummary::has_features)
        .collect::<Vec<_>>();

    match options.format {
        OutputFormat::Markdown => print_markdown(&options.root, &summaries),
        OutputFormat::Json => print_json(&options.root, &summaries)?,
    }
    Ok(())
}

fn script_args() -> impl Iterator<Item = OsString> {
    env::args_os().filter(|arg| arg != "--")
}

fn inspect_fixture(path: PathBuf) -> FixtureSummary {
    let mut summary = FixtureSummary {
        path: path.clone(),
        ..FixtureSummary::default()
    };
    let document = match gltf::import(&path) {
        Ok((document, _, _)) => document,
        Err(err) => {
            summary.load_error = Some(err.to_string());
            return summary;
        }
    };

    for material in document.materials() {
        if let Some(mtoon) = material.extension_value("VRMC_materials_mtoon") {
            summary.vrm1_materials += 1;
            summarize_vrm1_mtoon(&mut summary, &material, mtoon);
        }
    }

    if let Some(vrm0) = root_extension_value(&document, "VRM") {
        summarize_vrm0_materials(&mut summary, &vrm0);
    }

    summary
}

fn summarize_vrm1_mtoon(
    summary: &mut FixtureSummary,
    material: &gltf::Material<'_>,
    mtoon: &Value,
) {
    match material.alpha_mode() {
        gltf::material::AlphaMode::Blend => summary.alpha_blend += 1,
        gltf::material::AlphaMode::Mask => summary.alpha_mask += 1,
        gltf::material::AlphaMode::Opaque => {}
    }
    if bool_field(mtoon, "transparentWithZWrite") {
        summary.transparent_z_write += 1;
    }
    match outline_mode(mtoon.get("outlineWidthMode").and_then(Value::as_str)) {
        OutlineMode::None => summary.outline_none += 1,
        OutlineMode::World => summary.outline_world += 1,
        OutlineMode::Screen => summary.outline_screen += 1,
    }
    if has_texture(mtoon, "outlineWidthMultiplyTexture") {
        summary.outline_width_textures += 1;
    }
    if has_texture(mtoon, "shadeMultiplyTexture") {
        summary.shade_textures += 1;
    }
    if has_texture(mtoon, "normalTexture") || material.normal_texture().is_some() {
        summary.normal_textures += 1;
    }
    if has_texture(mtoon, "matcapTexture") {
        summary.matcap_textures += 1;
    }
    if has_texture(mtoon, "rimMultiplyTexture") {
        summary.rim_textures += 1;
    }
    if has_texture(mtoon, "shadingShiftTexture") {
        summary.shading_shift_textures += 1;
    }
    if float_field(mtoon, "uvAnimationScrollXSpeedFactor") != 0.0
        || float_field(mtoon, "uvAnimationScrollYSpeedFactor") != 0.0
        || float_field(mtoon, "uvAnimationRotationSpeedFactor") != 0.0
    {
        summary.uv_animation += 1;
    }
    if has_texture(mtoon, "uvAnimationMaskTexture") {
        summary.uv_animation_mask_textures += 1;
    }
}

fn summarize_vrm0_materials(summary: &mut FixtureSummary, vrm0: &Value) {
    let Some(materials) = vrm0.get("materialProperties").and_then(Value::as_array) else {
        return;
    };
    for material in materials {
        let shader = material
            .get("shader")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !shader.contains("MToon") {
            continue;
        }
        summary.vrm0_materials += 1;

        let float_properties = material.get("floatProperties");
        let texture_properties = material.get("textureProperties");
        let keyword_map = material.get("keywordMap");
        if bool_map_field(keyword_map, "_ALPHABLEND_ON") {
            summary.alpha_blend += 1;
        }
        if bool_map_field(keyword_map, "_ALPHATEST_ON") {
            summary.alpha_mask += 1;
        }
        if bool_map_field(keyword_map, "_ALPHABLEND_ON")
            && float_map_field(float_properties, "_ZWrite") > 0.0
        {
            summary.transparent_z_write += 1;
        }
        match float_map_field(float_properties, "_OutlineWidthMode") as i32 {
            1 => summary.outline_world += 1,
            2 => summary.outline_screen += 1,
            _ => summary.outline_none += 1,
        }
        if texture_map_field(texture_properties, "_OutlineWidthTexture") {
            summary.outline_width_textures += 1;
        }
        if texture_map_field(texture_properties, "_ShadeTexture") {
            summary.shade_textures += 1;
        }
        if texture_map_field(texture_properties, "_BumpMap") {
            summary.normal_textures += 1;
        }
        if texture_map_field(texture_properties, "_SphereAdd") {
            summary.matcap_textures += 1;
        }
        if texture_map_field(texture_properties, "_RimTexture") {
            summary.rim_textures += 1;
        }
        if float_map_field(float_properties, "_UvAnimScrollX") != 0.0
            || float_map_field(float_properties, "_UvAnimScrollY") != 0.0
            || float_map_field(float_properties, "_UvAnimRotation") != 0.0
        {
            summary.uv_animation += 1;
        }
        if texture_map_field(texture_properties, "_UvAnimMaskTexture") {
            summary.uv_animation_mask_textures += 1;
        }
    }
}

fn root_extension_value(document: &gltf::Document, name: &str) -> Option<Value> {
    let extensions = document.as_json().extensions.as_ref()?;
    serde_json::to_value(extensions).ok()?.get(name).cloned()
}

fn outline_mode(value: Option<&str>) -> OutlineMode {
    match value {
        Some("worldCoordinates") => OutlineMode::World,
        Some("screenCoordinates") => OutlineMode::Screen,
        _ => OutlineMode::None,
    }
}

fn has_texture(value: &Value, field: &str) -> bool {
    value.get(field).and_then(|texture| texture.get("index")).is_some()
}

fn bool_field(value: &Value, field: &str) -> bool {
    value.get(field).and_then(Value::as_bool).unwrap_or(false)
}

fn float_field(value: &Value, field: &str) -> f64 {
    value.get(field).and_then(Value::as_f64).unwrap_or(0.0)
}

fn bool_map_field(value: Option<&Value>, field: &str) -> bool {
    value
        .and_then(|value| value.get(field))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn float_map_field(value: Option<&Value>, field: &str) -> f64 {
    value
        .and_then(|value| value.get(field))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

fn texture_map_field(value: Option<&Value>, field: &str) -> bool {
    value
        .and_then(|value| value.get(field))
        .and_then(Value::as_i64)
        .is_some_and(|index| index >= 0)
}

fn print_markdown(root: &Path, summaries: &[FixtureSummary]) {
    println!("# MToon Fixture Feature Inventory");
    println!();
    println!("Root: `{}`", root.display());
    println!();
    println!("| Fixture | MToon | outline none/world/screen | alpha blend/mask/zwrite | texture slots | UV anim | Notes |");
    println!("| --- | ---: | ---: | ---: | --- | ---: | --- |");
    for summary in summaries {
        println!(
            "| `{}` | {} | {}/{}/{} | {}/{}/{} | shade={} normal={} matcap={} rim={} shift={} outlineWidth={} uvMask={} | {} | {} |",
            summary.path.display(),
            summary.mtoon_total(),
            summary.outline_none,
            summary.outline_world,
            summary.outline_screen,
            summary.alpha_blend,
            summary.alpha_mask,
            summary.transparent_z_write,
            summary.shade_textures,
            summary.normal_textures,
            summary.matcap_textures,
            summary.rim_textures,
            summary.shading_shift_textures,
            summary.outline_width_textures,
            summary.uv_animation_mask_textures,
            summary.uv_animation,
            summary
                .load_error
                .as_deref()
                .unwrap_or_else(|| if summary.vrm0_materials > 0 { "VRM0" } else { "VRM1" })
        );
    }
    let totals = totals(summaries);
    println!();
    println!(
        "Totals: MToon={} outline screen={} transparent z-write={} uv-animation={} normal textures={} matcap textures={}",
        totals.mtoon_total(),
        totals.outline_screen,
        totals.transparent_z_write,
        totals.uv_animation,
        totals.normal_textures,
        totals.matcap_textures
    );
}

fn print_json(root: &Path, summaries: &[FixtureSummary]) -> Result<(), String> {
    let values = summaries
        .iter()
        .map(|summary| {
            serde_json::json!({
                "path": summary.path,
                "vrm0Materials": summary.vrm0_materials,
                "vrm1Materials": summary.vrm1_materials,
                "mtoonMaterials": summary.mtoon_total(),
                "outline": {
                    "none": summary.outline_none,
                    "world": summary.outline_world,
                    "screen": summary.outline_screen,
                    "widthTextures": summary.outline_width_textures
                },
                "alpha": {
                    "blend": summary.alpha_blend,
                    "mask": summary.alpha_mask,
                    "transparentZWrite": summary.transparent_z_write
                },
                "textures": {
                    "shade": summary.shade_textures,
                    "normal": summary.normal_textures,
                    "matcap": summary.matcap_textures,
                    "rim": summary.rim_textures,
                    "shadingShift": summary.shading_shift_textures,
                    "uvAnimationMask": summary.uv_animation_mask_textures
                },
                "uvAnimation": summary.uv_animation,
                "loadError": summary.load_error,
            })
        })
        .collect::<Vec<_>>();
    let output = serde_json::json!({
        "root": root,
        "totals": summary_json(&totals(summaries)),
        "fixtures": values,
    });
    serde_json::to_writer_pretty(std::io::stdout(), &output).map_err(|err| err.to_string())?;
    println!();
    Ok(())
}

fn summary_json(summary: &FixtureSummary) -> Value {
    serde_json::json!({
        "vrm0Materials": summary.vrm0_materials,
        "vrm1Materials": summary.vrm1_materials,
        "mtoonMaterials": summary.mtoon_total(),
        "outlineScreen": summary.outline_screen,
        "transparentZWrite": summary.transparent_z_write,
        "uvAnimation": summary.uv_animation,
        "normalTextures": summary.normal_textures,
        "matcapTextures": summary.matcap_textures,
    })
}

fn totals(summaries: &[FixtureSummary]) -> FixtureSummary {
    summaries.iter().fold(FixtureSummary::default(), |mut total, summary| {
        total.vrm0_materials += summary.vrm0_materials;
        total.vrm1_materials += summary.vrm1_materials;
        total.outline_none += summary.outline_none;
        total.outline_world += summary.outline_world;
        total.outline_screen += summary.outline_screen;
        total.outline_width_textures += summary.outline_width_textures;
        total.alpha_blend += summary.alpha_blend;
        total.alpha_mask += summary.alpha_mask;
        total.transparent_z_write += summary.transparent_z_write;
        total.shade_textures += summary.shade_textures;
        total.normal_textures += summary.normal_textures;
        total.matcap_textures += summary.matcap_textures;
        total.rim_textures += summary.rim_textures;
        total.shading_shift_textures += summary.shading_shift_textures;
        total.uv_animation += summary.uv_animation;
        total.uv_animation_mask_textures += summary.uv_animation_mask_textures;
        total
    })
}

fn supported_fixtures_under(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect_supported_fixtures(root, &mut paths);
    paths.sort();
    paths
}

fn collect_supported_fixtures(path: &Path, paths: &mut Vec<PathBuf>) {
    if path.is_file() {
        if is_supported_fixture(path) {
            paths.push(path.to_owned());
        }
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_supported_fixtures(&entry.path(), paths);
    }
}

fn is_supported_fixture(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "vrm" | "glb" | "gltf"
            )
        })
}
