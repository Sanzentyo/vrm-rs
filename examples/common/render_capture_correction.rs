use std::fs;
use std::io;
use std::path::Path;
use vrm_adapter::RenderOwnerSampleCorrectionPlan;

pub fn apply_owner_sample_correction_manifest(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &mut [u8],
) -> io::Result<usize> {
    let value =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path)?).map_err(io_other)?;
    let plan = RenderOwnerSampleCorrectionPlan::from_manifest_value(&value).map_err(io_other)?;
    plan.apply_rgba8(u64::from(width), u64::from(height), rgba)
        .map_err(io_other)
}

fn io_other(error: impl ToString) -> io::Error {
    io::Error::other(error.to_string())
}
