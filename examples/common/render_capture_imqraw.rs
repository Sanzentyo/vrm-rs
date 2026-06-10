use imq::{FrameOwned, PixelFormat, RawImageBundle, RawImageRecord, encode_imqraw_bundle};
use std::fs;
use std::io;
use std::path::Path;

pub fn write_imqraw_rgba8(
    path: &Path,
    label: impl Into<String>,
    tags: impl IntoIterator<Item = impl Into<String>>,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let frame = FrameOwned::packed_tight(rgba.to_vec(), width, height, PixelFormat::Rgba8)
        .map_err(|err| io::Error::other(format!("failed to create imqraw RGBA frame: {err}")))?;
    let record = RawImageRecord::new(
        Some(label.into()),
        tags.into_iter().map(Into::into).collect(),
        frame,
    );
    let bytes = encode_imqraw_bundle(&RawImageBundle::new(vec![record]))
        .map_err(|err| io::Error::other(format!("failed to encode imqraw bundle: {err}")))?;
    fs::write(path, bytes)?;
    Ok(())
}
