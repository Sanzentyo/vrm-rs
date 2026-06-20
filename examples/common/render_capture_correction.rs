use std::fs;
use std::io;
use std::path::Path;
use vrm_adapter::apply_render_rgba8_corrections_from_manifest_value;

pub fn apply_owner_sample_correction_manifest(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &mut [u8],
) -> io::Result<usize> {
    let value =
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path)?).map_err(io_other)?;
    apply_render_rgba8_corrections_from_manifest_value(
        u64::from(width),
        u64::from(height),
        rgba,
        &value,
    )
    .map_err(io_other)
}

fn io_other(error: impl ToString) -> io::Error {
    io::Error::other(error.to_string())
}
