//! OSC packet encoder.

use crate::{OscBundle, OscMessage, OscPacket, OscTime, OscType, Result};

/// Takes a reference to an OSC packet and returns a byte vector.
pub fn encode(packet: &OscPacket) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    encode_into(packet, &mut bytes).expect("Vec<u8> output is infallible");
    Ok(bytes)
}

/// Works like [`encode`], but prepends the packet size as a 32-bit big-endian
/// length, as required by OSC stream transports such as TCP.
pub fn encode_tcp(packet: &OscPacket) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    encode_into_tcp(packet, &mut bytes).expect("Vec<u8> output is infallible");
    Ok(bytes)
}

/// Writes an encoded OSC packet into an output sink.
pub fn encode_into<O: Output>(
    packet: &OscPacket,
    out: &mut O,
) -> std::result::Result<usize, O::Err> {
    match packet {
        OscPacket::Message(message) => encode_message(message, out),
        OscPacket::Bundle(bundle) => encode_bundle(bundle, out),
    }
}

/// Writes a TCP/stream length-prefixed OSC packet into an output sink.
pub fn encode_into_tcp<O: Output>(
    packet: &OscPacket,
    out: &mut O,
) -> std::result::Result<usize, O::Err> {
    let mark = out.mark(4)?;
    let length = encode_into(packet, out)?;
    out.place(mark, &(length as u32).to_be_bytes())?;
    Ok(length + 4)
}

fn encode_message<O: Output>(
    message: &OscMessage,
    out: &mut O,
) -> std::result::Result<usize, O::Err> {
    let mut written = encode_string_into(&message.addr, out)?;

    let mut tags = String::from(",");
    for arg in &message.args {
        append_arg_type(arg, &mut tags);
    }
    written += encode_string_into(tags, out)?;

    for arg in &message.args {
        written += encode_arg_data(arg, out)?;
    }

    Ok(written)
}

fn encode_bundle<O: Output>(bundle: &OscBundle, out: &mut O) -> std::result::Result<usize, O::Err> {
    let mut written = encode_string_into("#bundle", out)?;
    written += encode_time_tag_into(&bundle.timetag, out)?;

    for packet in &bundle.content {
        let mark = out.mark(4)?;
        let length = encode_into(packet, out)?;
        out.place(mark, &(length as u32).to_be_bytes())?;
        written += 4 + length;
    }

    Ok(written)
}

fn encode_arg_data<O: Output>(arg: &OscType, out: &mut O) -> std::result::Result<usize, O::Err> {
    match arg {
        OscType::Int(value) => out.write(&value.to_be_bytes()),
        OscType::Long(value) => out.write(&value.to_be_bytes()),
        OscType::Float(value) => out.write(&value.to_be_bytes()),
        OscType::Double(value) => out.write(&value.to_be_bytes()),
        OscType::Char(value) => out.write(&(*value as u32).to_be_bytes()),
        OscType::String(value) => encode_string_into(value, out),
        OscType::Blob(value) => {
            let padded_blob_length = pad(value.len() as u64) as usize;
            let padding = padded_blob_length - value.len();
            let mut written = out.write(&(value.len() as u32).to_be_bytes())?;
            written += out.write(value)?;
            if padding > 0 {
                written += out.write(&[0u8; 3][..padding])?;
            }
            Ok(written)
        }
        OscType::Time(time) => encode_time_tag_into(time, out),
        OscType::Midi(value) => out.write(&[value.port, value.status, value.data1, value.data2]),
        OscType::Color(value) => out.write(&[value.red, value.green, value.blue, value.alpha]),
        OscType::Bool(_) | OscType::Nil | OscType::Inf => Ok(0),
        OscType::Array(value) => {
            let mut written = 0;
            for nested in &value.content {
                written += encode_arg_data(nested, out)?;
            }
            Ok(written)
        }
    }
}

fn append_arg_type(arg: &OscType, tags: &mut String) {
    match arg {
        OscType::Int(_) => tags.push('i'),
        OscType::Long(_) => tags.push('h'),
        OscType::Float(_) => tags.push('f'),
        OscType::Double(_) => tags.push('d'),
        OscType::Char(_) => tags.push('c'),
        OscType::String(_) => tags.push('s'),
        OscType::Blob(_) => tags.push('b'),
        OscType::Time(_) => tags.push('t'),
        OscType::Midi(_) => tags.push('m'),
        OscType::Color(_) => tags.push('r'),
        OscType::Bool(true) => tags.push('T'),
        OscType::Bool(false) => tags.push('F'),
        OscType::Nil => tags.push('N'),
        OscType::Inf => tags.push('I'),
        OscType::Array(value) => {
            tags.push('[');
            for nested in &value.content {
                append_arg_type(nested, tags);
            }
            tags.push(']');
        }
    }
}

/// Null-terminates a string and pads it to a 4-byte boundary.
pub fn encode_string<S: Into<String>>(value: S) -> Vec<u8> {
    let mut bytes = value.into().into_bytes();
    let new_len = pad(bytes.len() as u64 + 1) as usize;
    bytes.resize(new_len, 0);
    bytes
}

/// Writes a null-terminated, 4-byte-padded OSC string into an output sink.
pub fn encode_string_into<S: AsRef<str>, O: Output>(
    value: S,
    out: &mut O,
) -> std::result::Result<usize, O::Err> {
    let value = value.as_ref();
    let padded_len = pad(value.len() as u64 + 1) as usize;
    let padding = padded_len - value.len();
    out.write(value.as_bytes())?;
    out.write(&[0u8; 4][..padding])?;
    Ok(value.len() + padding)
}

/// Returns `pos` rounded up to a 4-byte boundary.
#[must_use]
pub fn pad(pos: u64) -> u64 {
    match pos % 4 {
        0 => pos,
        remainder => pos + (4 - remainder),
    }
}

fn encode_time_tag_into<O: Output>(
    time: &OscTime,
    out: &mut O,
) -> std::result::Result<usize, O::Err> {
    out.write(&time.seconds.to_be_bytes())?;
    out.write(&time.fractional.to_be_bytes())?;
    Ok(8)
}

/// A sink for encoded OSC output.
pub trait Output {
    /// Error type returned by the output sink.
    type Err;
    /// Opaque marker for a fixed-width region that can be backfilled later.
    type Mark;

    /// Writes all bytes from `data` and returns the number of bytes written.
    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, Self::Err>;

    /// Reserves `size` bytes and returns a mark that can later be filled with
    /// [`Output::place`].
    fn mark(&mut self, size: usize) -> std::result::Result<Self::Mark, Self::Err>;

    /// Backfills a previously reserved mark with `data`.
    fn place(&mut self, mark: Self::Mark, data: &[u8]) -> std::result::Result<(), Self::Err>;
}

impl Output for Vec<u8> {
    type Err = std::convert::Infallible;
    type Mark = (usize, usize);

    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, Self::Err> {
        self.extend_from_slice(data);
        Ok(data.len())
    }

    fn mark(&mut self, size: usize) -> std::result::Result<Self::Mark, Self::Err> {
        let start = self.len();
        let end = start + size;
        self.resize(end, 0);
        Ok((start, end))
    }

    fn place(
        &mut self,
        (start, end): Self::Mark,
        data: &[u8],
    ) -> std::result::Result<(), Self::Err> {
        assert_eq!(
            end - start,
            data.len(),
            "mark size and data size must match"
        );
        self[start..end].copy_from_slice(data);
        Ok(())
    }
}

/// Adapter that allows encoding directly into any `Seek + Write` sink.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WriteOutput<W>(pub W);

impl<W: std::io::Seek + std::io::Write> Output for WriteOutput<W> {
    type Err = std::io::Error;
    type Mark = u64;

    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, Self::Err> {
        std::io::Write::write_all(&mut self.0, data).map(|()| data.len())
    }

    fn mark(&mut self, size: usize) -> std::result::Result<Self::Mark, Self::Err> {
        let pos = std::io::Seek::stream_position(&mut self.0)?;
        let mut left = size;
        while left > 0 {
            let amount = left.min(8);
            std::io::Write::write_all(&mut self.0, &[0; 8][..amount])?;
            left -= amount;
        }
        Ok(pos)
    }

    fn place(&mut self, mark: Self::Mark, data: &[u8]) -> std::result::Result<(), Self::Err> {
        let old_pos = std::io::Seek::stream_position(&mut self.0)?;
        std::io::Seek::seek(&mut self.0, std::io::SeekFrom::Start(mark))?;
        std::io::Write::write_all(&mut self.0, data)?;
        std::io::Seek::seek(&mut self.0, std::io::SeekFrom::Start(old_pos))?;
        Ok(())
    }
}
