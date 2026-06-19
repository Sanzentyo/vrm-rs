use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceLimits {
    pub max_data_uri_bytes: usize,
    pub max_file_bytes: usize,
    pub max_decoded_bytes: usize,
    pub allow_file_paths: bool,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_data_uri_bytes: 64 * 1024 * 1024,
            max_file_bytes: 256 * 1024 * 1024,
            max_decoded_bytes: 512 * 1024 * 1024,
            allow_file_paths: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceData {
    pub source: ResourceSource,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceSource {
    DataUri { media_type: Option<String> },
    RelativeFile { path: PathBuf },
}

pub trait ResourceReader {
    fn read_relative(&self, path: &Path, max_bytes: usize) -> Result<Vec<u8>, ResourceError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileResourceReader {
    base_dir: PathBuf,
}

impl FileResourceReader {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}

impl ResourceReader for FileResourceReader {
    fn read_relative(&self, path: &Path, max_bytes: usize) -> Result<Vec<u8>, ResourceError> {
        read_limited_file(&self.base_dir.join(path), max_bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshCodec {
    Draco,
    Meshopt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureCodec {
    Ktx2Basis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressedMeshPayload {
    pub codec: MeshCodec,
    pub bytes: Vec<u8>,
    pub declared_decoded_bytes: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedMeshPayload {
    pub bytes: Vec<u8>,
    pub vertex_count: usize,
    pub index_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressedTexturePayload {
    pub codec: TextureCodec,
    pub bytes: Vec<u8>,
    pub declared_decoded_bytes: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedTexturePayload {
    pub format: TextureOutputFormat,
    pub width: u32,
    pub height: u32,
    pub mip_levels: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureOutputFormat {
    Rgba8Unorm,
    Rgba8Srgb,
    Bc7RgbaUnorm,
    Bc7RgbaSrgb,
    Etc2Rgba8,
    Astc4x4Rgba,
}

pub trait MeshCodecProvider {
    fn codec(&self) -> MeshCodec;
    fn decode_mesh(
        &self,
        payload: &CompressedMeshPayload,
    ) -> Result<DecodedMeshPayload, ResourceError>;
}

pub trait TextureCodecProvider {
    fn codec(&self) -> TextureCodec;
    fn decode_texture(
        &self,
        payload: &CompressedTexturePayload,
    ) -> Result<DecodedTexturePayload, ResourceError>;
}

#[derive(Default)]
pub struct CodecRegistry {
    mesh: HashMap<MeshCodec, Box<dyn MeshCodecProvider>>,
    texture: HashMap<TextureCodec, Box<dyn TextureCodecProvider>>,
    limits: ResourceLimits,
}

impl CodecRegistry {
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    pub fn limits(&self) -> ResourceLimits {
        self.limits
    }

    pub fn register_mesh<P>(&mut self, provider: P)
    where
        P: MeshCodecProvider + 'static,
    {
        self.mesh.insert(provider.codec(), Box::new(provider));
    }

    pub fn register_texture<P>(&mut self, provider: P)
    where
        P: TextureCodecProvider + 'static,
    {
        self.texture.insert(provider.codec(), Box::new(provider));
    }

    pub fn mesh_provider(&self, codec: MeshCodec) -> Option<&dyn MeshCodecProvider> {
        self.mesh.get(&codec).map(Box::as_ref)
    }

    pub fn texture_provider(&self, codec: TextureCodec) -> Option<&dyn TextureCodecProvider> {
        self.texture.get(&codec).map(Box::as_ref)
    }

    pub fn resolve_resource<R>(&self, uri: &str, reader: &R) -> Result<ResourceData, ResourceError>
    where
        R: ResourceReader,
    {
        resolve_resource_uri(uri, reader, self.limits)
    }

    pub fn decode_mesh(
        &self,
        payload: &CompressedMeshPayload,
    ) -> Result<DecodedMeshPayload, ResourceError> {
        ensure_payload_size(
            payload.bytes.len(),
            self.limits.max_file_bytes,
            "mesh codec input",
        )?;
        if let Some(size) = payload.declared_decoded_bytes {
            ensure_payload_size(size, self.limits.max_decoded_bytes, "mesh codec output")?;
        }
        let provider = self
            .mesh
            .get(&payload.codec)
            .ok_or(ResourceError::MissingMeshCodec {
                codec: payload.codec,
            })?;
        let decoded = provider.decode_mesh(payload)?;
        ensure_payload_size(
            decoded.bytes.len(),
            self.limits.max_decoded_bytes,
            "mesh codec output",
        )?;
        Ok(decoded)
    }

    pub fn decode_texture(
        &self,
        payload: &CompressedTexturePayload,
    ) -> Result<DecodedTexturePayload, ResourceError> {
        ensure_payload_size(
            payload.bytes.len(),
            self.limits.max_file_bytes,
            "texture codec input",
        )?;
        if let Some(size) = payload.declared_decoded_bytes {
            ensure_payload_size(size, self.limits.max_decoded_bytes, "texture codec output")?;
        }
        let provider =
            self.texture
                .get(&payload.codec)
                .ok_or(ResourceError::MissingTextureCodec {
                    codec: payload.codec,
                })?;
        let decoded = provider.decode_texture(payload)?;
        ensure_payload_size(
            decoded.bytes.len(),
            self.limits.max_decoded_bytes,
            "texture codec output",
        )?;
        Ok(decoded)
    }
}

pub fn resolve_resource_uri<R>(
    uri: &str,
    reader: &R,
    limits: ResourceLimits,
) -> Result<ResourceData, ResourceError>
where
    R: ResourceReader,
{
    if let Some(data) = parse_data_uri(uri, limits)? {
        return Ok(data);
    }
    if !limits.allow_file_paths {
        return Err(ResourceError::FilePathsDisabled);
    }
    let path = sanitize_relative_path(uri)?;
    let bytes = reader.read_relative(&path, limits.max_file_bytes)?;
    Ok(ResourceData {
        source: ResourceSource::RelativeFile { path },
        bytes,
    })
}

#[derive(Debug, Error)]
pub enum ResourceError {
    #[error("resource file paths are disabled")]
    FilePathsDisabled,
    #[error("resource path must be relative and stay within the base directory: {path}")]
    UnsafePath { path: String },
    #[error("invalid data URI: {message}")]
    InvalidDataUri { message: String },
    #[error("{kind} exceeds limit: {actual} bytes > {limit} bytes")]
    SizeLimit {
        kind: &'static str,
        actual: usize,
        limit: usize,
    },
    #[error("missing mesh codec provider: {codec:?}")]
    MissingMeshCodec { codec: MeshCodec },
    #[error("missing texture codec provider: {codec:?}")]
    MissingTextureCodec { codec: TextureCodec },
    #[error("codec provider failed: {message}")]
    CodecProvider { message: String },
    #[error(transparent)]
    Io(#[from] io::Error),
}

fn parse_data_uri(
    uri: &str,
    limits: ResourceLimits,
) -> Result<Option<ResourceData>, ResourceError> {
    let Some(rest) = uri.strip_prefix("data:") else {
        return Ok(None);
    };
    let (metadata, payload) =
        rest.split_once(',')
            .ok_or_else(|| ResourceError::InvalidDataUri {
                message: "missing comma separator".to_owned(),
            })?;
    let mut media_type = None;
    let mut is_base64 = false;
    for (index, part) in metadata.split(';').enumerate() {
        if part.eq_ignore_ascii_case("base64") {
            is_base64 = true;
        } else if index == 0 && !part.is_empty() {
            media_type = Some(part.to_owned());
        } else if !part.is_empty() {
            return Err(ResourceError::InvalidDataUri {
                message: format!("unsupported data URI parameter: {part}"),
            });
        }
    }
    let bytes = if is_base64 {
        decode_base64(payload)?
    } else {
        percent_decode(payload)?
    };
    ensure_payload_size(bytes.len(), limits.max_data_uri_bytes, "data URI")?;
    Ok(Some(ResourceData {
        source: ResourceSource::DataUri { media_type },
        bytes,
    }))
}

fn sanitize_relative_path(uri: &str) -> Result<PathBuf, ResourceError> {
    let path = Path::new(uri);
    if path.is_absolute() {
        return Err(ResourceError::UnsafePath {
            path: uri.to_owned(),
        });
    }
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => output.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ResourceError::UnsafePath {
                    path: uri.to_owned(),
                });
            }
        }
    }
    if output.as_os_str().is_empty() {
        return Err(ResourceError::UnsafePath {
            path: uri.to_owned(),
        });
    }
    Ok(output)
}

fn read_limited_file(path: &Path, limit: usize) -> Result<Vec<u8>, ResourceError> {
    let file = File::open(path)?;
    if let Ok(metadata) = file.metadata() {
        ensure_payload_size(metadata.len() as usize, limit, "resource file")?;
    }
    let mut bytes = Vec::new();
    let mut limited = file.take(limit as u64 + 1);
    limited.read_to_end(&mut bytes)?;
    ensure_payload_size(bytes.len(), limit, "resource file")?;
    Ok(bytes)
}

fn ensure_payload_size(
    actual: usize,
    limit: usize,
    kind: &'static str,
) -> Result<(), ResourceError> {
    if actual > limit {
        Err(ResourceError::SizeLimit {
            kind,
            actual,
            limit,
        })
    } else {
        Ok(())
    }
}

fn percent_decode(payload: &str) -> Result<Vec<u8>, ResourceError> {
    let bytes = payload.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(ResourceError::InvalidDataUri {
                    message: "truncated percent escape".to_owned(),
                });
            }
            let high = hex_value(bytes[index + 1])?;
            let low = hex_value(bytes[index + 2])?;
            output.push(high << 4 | low);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    Ok(output)
}

fn hex_value(byte: u8) -> Result<u8, ResourceError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(ResourceError::InvalidDataUri {
            message: "invalid percent escape".to_owned(),
        }),
    }
}

fn decode_base64(payload: &str) -> Result<Vec<u8>, ResourceError> {
    let cleaned = payload
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if cleaned.len() % 4 != 0 {
        return Err(ResourceError::InvalidDataUri {
            message: "base64 payload length is not a multiple of four".to_owned(),
        });
    }
    let mut output = Vec::with_capacity(cleaned.len() / 4 * 3);
    for chunk in cleaned.chunks_exact(4) {
        let a = base64_value(chunk[0])?;
        let b = base64_value(chunk[1])?;
        let c = base64_value_or_padding(chunk[2])?;
        let d = base64_value_or_padding(chunk[3])?;
        output.push((a << 2) | (b >> 4));
        if let Some(c) = c {
            output.push((b << 4) | (c >> 2));
            if let Some(d) = d {
                output.push((c << 6) | d);
            }
        } else if d.is_some() {
            return Err(ResourceError::InvalidDataUri {
                message: "base64 padding is malformed".to_owned(),
            });
        }
    }
    Ok(output)
}

fn base64_value_or_padding(byte: u8) -> Result<Option<u8>, ResourceError> {
    if byte == b'=' {
        Ok(None)
    } else {
        base64_value(byte).map(Some)
    }
}

fn base64_value(byte: u8) -> Result<u8, ResourceError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(ResourceError::InvalidDataUri {
            message: "invalid base64 character".to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct MemoryReader {
        bytes: HashMap<PathBuf, Vec<u8>>,
    }

    impl ResourceReader for MemoryReader {
        fn read_relative(&self, path: &Path, max_bytes: usize) -> Result<Vec<u8>, ResourceError> {
            let bytes = self.bytes.get(path).cloned().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, path.display().to_string())
            })?;
            ensure_payload_size(bytes.len(), max_bytes, "resource file")?;
            Ok(bytes)
        }
    }

    struct EchoMeshCodec {
        calls: Cell<usize>,
    }

    impl MeshCodecProvider for EchoMeshCodec {
        fn codec(&self) -> MeshCodec {
            MeshCodec::Meshopt
        }

        fn decode_mesh(
            &self,
            payload: &CompressedMeshPayload,
        ) -> Result<DecodedMeshPayload, ResourceError> {
            self.calls.set(self.calls.get() + 1);
            Ok(DecodedMeshPayload {
                bytes: payload.bytes.clone(),
                vertex_count: 3,
                index_count: 3,
            })
        }
    }

    struct EchoTextureCodec;

    impl TextureCodecProvider for EchoTextureCodec {
        fn codec(&self) -> TextureCodec {
            TextureCodec::Ktx2Basis
        }

        fn decode_texture(
            &self,
            payload: &CompressedTexturePayload,
        ) -> Result<DecodedTexturePayload, ResourceError> {
            Ok(DecodedTexturePayload {
                format: TextureOutputFormat::Rgba8Srgb,
                width: 1,
                height: 1,
                mip_levels: 1,
                bytes: payload.bytes.clone(),
            })
        }
    }

    #[test]
    fn resolves_base64_and_percent_encoded_data_uris() {
        let registry = CodecRegistry::default();
        let reader = MemoryReader::default();

        let base64 = registry
            .resolve_resource("data:text/plain;base64,dmVydGV4", &reader)
            .unwrap();
        assert_eq!(base64.bytes, b"vertex");
        assert_eq!(
            base64.source,
            ResourceSource::DataUri {
                media_type: Some("text/plain".to_owned())
            }
        );

        let plain = registry
            .resolve_resource("data:,hello%20vrm", &reader)
            .unwrap();
        assert_eq!(plain.bytes, b"hello vrm");
    }

    #[test]
    fn rejects_path_traversal_absolute_paths_and_disabled_files() {
        let reader = MemoryReader::default();
        let registry = CodecRegistry::default();

        assert!(matches!(
            registry.resolve_resource("../secret.bin", &reader),
            Err(ResourceError::UnsafePath { .. })
        ));
        assert!(matches!(
            registry.resolve_resource("C:/secret.bin", &reader),
            Err(ResourceError::UnsafePath { .. })
        ));

        let locked = CodecRegistry::new(ResourceLimits {
            allow_file_paths: false,
            ..ResourceLimits::default()
        });
        assert!(matches!(
            locked.resolve_resource("textures/albedo.png", &reader),
            Err(ResourceError::FilePathsDisabled)
        ));
    }

    #[test]
    fn resolves_relative_files_through_reader_with_size_limit() {
        let mut reader = MemoryReader::default();
        reader
            .bytes
            .insert(PathBuf::from("textures/albedo.png"), vec![1, 2, 3]);
        let registry = CodecRegistry::new(ResourceLimits {
            max_file_bytes: 3,
            ..ResourceLimits::default()
        });

        let resolved = registry
            .resolve_resource("./textures/albedo.png", &reader)
            .unwrap();

        assert_eq!(resolved.bytes, vec![1, 2, 3]);
        assert_eq!(
            resolved.source,
            ResourceSource::RelativeFile {
                path: PathBuf::from("textures/albedo.png")
            }
        );

        let tiny = CodecRegistry::new(ResourceLimits {
            max_file_bytes: 2,
            ..ResourceLimits::default()
        });
        assert!(matches!(
            tiny.resolve_resource("textures/albedo.png", &reader),
            Err(ResourceError::SizeLimit {
                kind: "resource file",
                actual: 3,
                limit: 2,
            })
        ));
    }

    #[test]
    fn missing_codecs_are_explicit_errors() {
        let registry = CodecRegistry::default();

        assert!(matches!(
            registry.decode_mesh(&CompressedMeshPayload {
                codec: MeshCodec::Draco,
                bytes: vec![1, 2, 3],
                declared_decoded_bytes: None,
            }),
            Err(ResourceError::MissingMeshCodec {
                codec: MeshCodec::Draco
            })
        ));
        assert!(matches!(
            registry.decode_texture(&CompressedTexturePayload {
                codec: TextureCodec::Ktx2Basis,
                bytes: vec![1, 2, 3],
                declared_decoded_bytes: None,
            }),
            Err(ResourceError::MissingTextureCodec {
                codec: TextureCodec::Ktx2Basis
            })
        ));
    }

    #[test]
    fn registered_codecs_decode_and_decoded_size_is_limited() {
        let mut registry = CodecRegistry::new(ResourceLimits {
            max_decoded_bytes: 4,
            ..ResourceLimits::default()
        });
        registry.register_mesh(EchoMeshCodec {
            calls: Cell::new(0),
        });
        registry.register_texture(EchoTextureCodec);

        let mesh = registry
            .decode_mesh(&CompressedMeshPayload {
                codec: MeshCodec::Meshopt,
                bytes: vec![1, 2, 3],
                declared_decoded_bytes: Some(3),
            })
            .unwrap();
        assert_eq!(mesh.vertex_count, 3);
        assert_eq!(mesh.bytes, vec![1, 2, 3]);

        let texture = registry
            .decode_texture(&CompressedTexturePayload {
                codec: TextureCodec::Ktx2Basis,
                bytes: vec![9, 8, 7, 6],
                declared_decoded_bytes: Some(4),
            })
            .unwrap();
        assert_eq!(texture.format, TextureOutputFormat::Rgba8Srgb);

        assert!(matches!(
            registry.decode_mesh(&CompressedMeshPayload {
                codec: MeshCodec::Meshopt,
                bytes: vec![1, 2, 3],
                declared_decoded_bytes: Some(5),
            }),
            Err(ResourceError::SizeLimit {
                kind: "mesh codec output",
                actual: 5,
                limit: 4,
            })
        ));
    }
}
