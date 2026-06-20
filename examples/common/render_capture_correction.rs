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
    let coverage = plan.surface_coverage(surfaces);
    serde_json::json!({
        "manifest": path.to_string_lossy(),
        "entryCount": coverage.entry_count,
        "surfaceCount": coverage.surface_count,
        "matchedEntryCount": coverage.matched_entry_count,
        "unmatchedEntryCount": coverage.unmatched_entry_count,
        "matchedSurfaceCount": coverage.matched_surface_count,
        "allEntriesResolved": coverage.all_entries_resolved(),
        "unmatchedSurfaces": coverage.unmatched_surfaces.into_iter().map(|surface| {
            serde_json::json!({
                "materialName": surface.material_name(),
                "triangle": surface.triangle(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn io_other(error: impl ToString) -> io::Error {
    io::Error::other(error.to_string())
}
