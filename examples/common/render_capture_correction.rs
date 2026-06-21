use std::fs;
use std::io;
use std::path::Path;
use vrm_adapter::{
    RenderOwnerSampleCorrectionPlan, RenderOwnerSampleDrawKey, RenderOwnerSampleSurfaceOverride,
    RenderOwnerSurfaceKey,
};

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
    draws: impl IntoIterator<Item = RenderOwnerSampleDrawKey>,
) -> serde_json::Value {
    let surfaces = surfaces.into_iter().collect::<Vec<_>>();
    let draws = draws.into_iter().collect::<Vec<_>>();
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
                "entries": surface.overrides().map(|entry| entry_json(&surface.surface, entry)).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "drawSelections": draws.iter().map(|draw| {
            let entries = plan.entries().iter()
                .filter(|entry| entry.sample_geometry.is_some())
                .filter(|entry| entry.matches_draw(draw))
                .collect::<Vec<_>>();
            serde_json::json!({
                "draw": draw_json(draw),
                "entryCount": entries.len(),
                "entries": entries.iter().map(|entry| {
                    entry_json(entry.sample.surface(), RenderOwnerSampleSurfaceOverride::from(*entry))
                }).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "unmatchedEntries": selection.unmatched_entries.iter().map(|entry| {
            entry_json(entry.sample.surface(), RenderOwnerSampleSurfaceOverride::from(entry))
        }).collect::<Vec<_>>(),
        "unmatchedSurfaces": coverage.unmatched_surfaces.into_iter().map(|surface| {
            surface_json(&surface)
        }).collect::<Vec<_>>(),
    })
}

fn draw_json(draw: &RenderOwnerSampleDrawKey) -> serde_json::Value {
    serde_json::json!({
        "node": draw.node,
        "mesh": draw.mesh,
        "primitive": draw.primitive,
        "pass": draw.pass.as_str(),
        "key": format!(
            "node{}/mesh{}/prim{}/{}",
            draw.node,
            draw.mesh,
            draw.primitive,
            draw.pass.as_str(),
        ),
    })
}

fn surface_json(surface: &RenderOwnerSurfaceKey) -> serde_json::Value {
    serde_json::json!({
        "materialName": surface.material_name(),
        "triangle": surface.triangle(),
    })
}

fn entry_json(
    surface: &RenderOwnerSurfaceKey,
    entry: RenderOwnerSampleSurfaceOverride,
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "pixel": entry.pixel.to_pair(),
        "sample": entry.sample.to_pair(),
        "rgba": entry.replacement_rgba,
        "selectionSource": entry.selection_source.map(|source| source.as_str()),
        "relationToExpected": entry.relation_to_expected.map(|relation| relation.as_str()),
        "surface": surface_json(surface),
    });
    if let Some(geometry) = entry.sample_geometry {
        value["sampleGeometry"] = serde_json::json!({
            "node": geometry.node,
            "mesh": geometry.mesh,
            "primitive": geometry.primitive,
            "triangle": geometry.triangle,
            "indices": geometry.indices,
            "barycentric": geometry.barycentric,
            "rawUv": geometry.raw_uv,
            "baseUv": geometry.base_uv,
            "depth": geometry.depth,
            "pass": geometry.pass.as_str(),
        });
    }
    value
}

fn io_other(error: impl ToString) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrm_adapter::RenderOwnerSamplePass;

    #[test]
    fn metadata_reports_draw_selections_from_sample_geometry() {
        let value = serde_json::json!({
            "corrections": [
                {
                    "x": 12,
                    "y": 34,
                    "rgba": [1, 2, 3, 255],
                    "surface": {"materialName": "body", "triangle": 7},
                    "sample": [0.5, 0.5],
                    "sample_geometry": {
                        "node": 1,
                        "mesh": 2,
                        "primitive": 3,
                        "triangle": 7,
                        "indices": [4, 5, 6],
                        "barycentric": [0.2, 0.3, 0.5],
                        "raw_uv": [0.25, 0.75],
                        "base_uv": [0.25, 0.75],
                        "depth": 0.42,
                        "pass": "base"
                    }
                }
            ]
        });
        let plan = RenderOwnerSampleCorrectionPlan::from_manifest_value(&value).unwrap();
        let metadata = owner_sample_correction_plan_metadata(
            Path::new("manifest.json"),
            &plan,
            [RenderOwnerSurfaceKey::new("body", 7)],
            [RenderOwnerSampleDrawKey::new(
                1,
                2,
                3,
                RenderOwnerSamplePass::Base,
            )],
        );

        assert_eq!(metadata["entryCount"], 1);
        assert_eq!(metadata["drawSelections"][0]["entryCount"], 1);
        assert_eq!(
            metadata["drawSelections"][0]["draw"]["key"],
            "node1/mesh2/prim3/base"
        );
        assert_eq!(
            metadata["drawSelections"][0]["entries"][0]["pixel"],
            serde_json::json!([12, 34])
        );
    }
}
