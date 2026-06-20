use serde_json::Value;
use std::fs;
use std::io;
use std::path::Path;
use vrm_adapter::{RenderPixel, RenderRgba8Correction, apply_render_rgba8_corrections};

pub fn apply_owner_sample_correction_manifest(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &mut [u8],
) -> io::Result<usize> {
    let value = serde_json::from_str::<Value>(&fs::read_to_string(path)?).map_err(io_other)?;
    let corrections = correction_values(&value)
        .ok_or_else(|| {
            io_other("owner sample correction manifest must be an array or contain corrections[]")
        })?
        .iter()
        .map(parse_correction)
        .collect::<io::Result<Vec<_>>>()?;
    apply_render_rgba8_corrections(u64::from(width), u64::from(height), rgba, &corrections)
        .map_err(io_other)
}

fn correction_values(value: &Value) -> Option<&[Value]> {
    value
        .as_array()
        .or_else(|| value.get("corrections").and_then(Value::as_array))
        .map(Vec::as_slice)
}

fn parse_correction(value: &Value) -> io::Result<RenderRgba8Correction> {
    let x = value
        .get("x")
        .and_then(Value::as_u64)
        .ok_or_else(|| io_other("correction.x must be a u64"))?;
    let y = value
        .get("y")
        .and_then(Value::as_u64)
        .ok_or_else(|| io_other("correction.y must be a u64"))?;
    let rgba = value
        .get("rgba")
        .or_else(|| value.get("replacementRgba"))
        .ok_or_else(|| io_other("correction.rgba must be present"))?;
    Ok(RenderRgba8Correction::new(
        RenderPixel::new(x, y),
        parse_rgba(rgba)?,
    ))
}

fn parse_rgba(value: &Value) -> io::Result<[u8; 4]> {
    let channels = value
        .as_array()
        .ok_or_else(|| io_other("correction.rgba must be an array"))?;
    if channels.len() != 4 {
        return Err(io_other("correction.rgba must have exactly four channels"));
    }
    let mut rgba = [0; 4];
    for (index, channel) in channels.iter().enumerate() {
        let channel = channel
            .as_u64()
            .ok_or_else(|| io_other("correction.rgba channels must be u8 integers"))?;
        rgba[index] = u8::try_from(channel).map_err(io_other)?;
    }
    Ok(rgba)
}

fn io_other(error: impl ToString) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correction_manifest_applies_object_and_array_forms() {
        let mut rgba = vec![0; 2 * 2 * 4];
        let object = serde_json::json!({
            "corrections": [
                {"x": 1, "y": 1, "rgba": [1, 2, 3, 255]}
            ]
        });
        let corrections = correction_values(&object)
            .unwrap()
            .iter()
            .map(parse_correction)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(
            apply_render_rgba8_corrections(2, 2, &mut rgba, &corrections).unwrap(),
            1
        );
        assert_eq!(&rgba[12..16], &[1, 2, 3, 255]);

        let array = serde_json::json!([
            {"x": 0, "y": 0, "replacementRgba": [9, 8, 7, 255]}
        ]);
        assert_eq!(
            parse_correction(correction_values(&array).unwrap().first().unwrap())
                .unwrap()
                .replacement_rgba,
            [9, 8, 7, 255]
        );
    }
}
