use std::fs;
use std::io;
use std::path::Path;
use vrm_adapter::{RenderOwnerSampleCorrectionPlan, RenderOwnerSurfaceKey};

pub fn load_owner_sample_correction_manifest(
    path: &Path,
) -> io::Result<RenderOwnerSampleCorrectionPlan> {
    let value =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path)?).map_err(io_other)?;
    RenderOwnerSampleCorrectionPlan::from_manifest_value(&value).map_err(io_other)
}

pub fn apply_owner_sample_correction_plan(
    plan: &RenderOwnerSampleCorrectionPlan,
    width: u32,
    height: u32,
    rgba: &mut [u8],
) -> io::Result<usize> {
    plan.apply_rgba8(u64::from(width), u64::from(height), rgba)
        .map_err(io_other)
}

pub fn owner_sample_correction_plan_metadata(
    path: &Path,
    plan: &RenderOwnerSampleCorrectionPlan,
    surfaces: impl IntoIterator<Item = RenderOwnerSurfaceKey>,
) -> serde_json::Value {
    let surfaces = surfaces.into_iter().collect::<Vec<_>>();
    let coverage = plan.surface_coverage(surfaces.iter());
    let selection = plan.surface_selection_plan(surfaces.iter());
    serde_json::json!({
        "manifest": path.to_string_lossy(),
        "entryCount": selection.entry_count(),
        "surfaceCount": coverage.surface_count,
        "matchedEntryCount": selection.matched_entry_count(),
        "unmatchedEntryCount": selection.unmatched_entry_count(),
        "matchedSurfaceCount": coverage.matched_surface_count,
        "allEntriesResolved": selection.all_entries_resolved(),
        "surfaceSelections": selection.surfaces.iter().map(|surface| {
            serde_json::json!({
                "surface": surface_json(&surface.surface),
                "entryCount": surface.entries.len(),
                "entries": surface.entries.iter().map(entry_json).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "unmatchedEntries": selection.unmatched_entries.iter().map(entry_json).collect::<Vec<_>>(),
        "unmatchedSurfaces": coverage.unmatched_surfaces.into_iter().map(|surface| {
            surface_json(&surface)
        }).collect::<Vec<_>>(),
    })
}

fn surface_json(surface: &RenderOwnerSurfaceKey) -> serde_json::Value {
    serde_json::json!({
        "materialName": surface.material_name(),
        "triangle": surface.triangle(),
    })
}

fn entry_json(entry: &vrm_adapter::RenderOwnerSampleCorrectionManifestEntry) -> serde_json::Value {
    serde_json::json!({
        "pixel": [entry.correction.pixel.x(), entry.correction.pixel.y()],
        "sample": entry.sample.sample().to_pair(),
        "rgba": entry.correction.replacement_rgba,
        "relationToExpected": entry.relation_to_expected.map(|relation| relation.as_str()),
        "surface": {
            "materialName": entry.sample.surface().material_name(),
            "triangle": entry.sample.surface().triangle(),
        },
    })
}

fn io_other(error: impl ToString) -> io::Error {
    io::Error::other(error.to_string())
}
