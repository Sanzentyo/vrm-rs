//! OSC packet decoder.

use crate::{
    OscArray, OscBundle, OscColor, OscError, OscMessage, OscMidiMessage, OscPacket, OscTime,
    OscType, Result,
};

/// Common Ethernet MTU size, kept for API familiarity with `rosc`.
pub const MTU: usize = 1536;

/// Decodes one OSC packet from a UDP datagram and returns the unconsumed suffix.
pub fn decode_udp(input: &[u8]) -> Result<(&[u8], OscPacket)> {
    let mut cursor = Cursor::new(input);
    let packet = decode_packet(&mut cursor)?;
    Ok((&input[cursor.pos..], packet))
}

/// Decodes one OSC packet from a stream-style, length-prefixed byte slice.
///
/// Returns `Ok((input, None))` when the length prefix or full packet body is
/// not available yet.
pub fn decode_tcp(input: &[u8]) -> Result<(&[u8], Option<OscPacket>)> {
    if input.len() < 4 {
        return Ok((input, None));
    }
    let packet_len =
        u32::from_be_bytes(input[0..4].try_into().expect("slice length checked")) as usize;
    if input.len() - 4 < packet_len {
        return Ok((input, None));
    }

    let packet_bytes = &input[4..4 + packet_len];
    let mut cursor = Cursor::new(packet_bytes);
    let packet = decode_packet(&mut cursor)?;
    if cursor.pos != packet_bytes.len() {
        return Err(OscError::BadPacket(
            "TCP packet length exceeds decoded packet length",
        ));
    }

    Ok((&input[4 + packet_len..], Some(packet)))
}

/// Decodes every complete length-prefixed OSC packet from a stream byte slice.
pub fn decode_tcp_vec(mut input: &[u8]) -> Result<(&[u8], Vec<OscPacket>)> {
    let mut packets = Vec::new();
    while !input.is_empty() {
        let (remainder, packet) = decode_tcp(input)?;
        let Some(packet) = packet else {
            return Ok((remainder, packets));
        };
        packets.push(packet);
        input = remainder;
    }
    Ok((input, packets))
}

fn decode_packet(cursor: &mut Cursor<'_>) -> Result<OscPacket> {
    if cursor.remaining().is_empty() {
        return Err(OscError::BadPacket("Empty packet."));
    }

    let addr = cursor.read_padded_string()?;
    match addr.as_bytes().first().copied() {
        Some(b'/') => decode_message(addr, cursor),
        Some(b'#') if addr == "#bundle" => decode_bundle(cursor),
        _ => Err(OscError::BadPacket("Invalid message address or bundle tag")),
    }
}

fn decode_message(addr: String, cursor: &mut Cursor<'_>) -> Result<OscPacket> {
    let raw_type_tags = cursor.read_padded_string()?;
    if raw_type_tags.is_empty() {
        return Ok(OscPacket::Message(OscMessage { addr, args: vec![] }));
    }
    if !raw_type_tags.starts_with(',') {
        return Err(OscError::BadMessage(
            "OSC type tag string must start with ','",
        ));
    }

    let args = read_osc_args(cursor, raw_type_tags.chars().skip(1))?;
    Ok(OscPacket::Message(OscMessage { addr, args }))
}

fn decode_bundle(cursor: &mut Cursor<'_>) -> Result<OscPacket> {
    let timetag = cursor.read_time_tag()?;
    let mut content = Vec::new();
    while !cursor.remaining().is_empty() {
        content.push(read_bundle_element(cursor)?);
    }
    Ok(OscPacket::Bundle(OscBundle { timetag, content }))
}

fn read_bundle_element(cursor: &mut Cursor<'_>) -> Result<OscPacket> {
    let elem_size = cursor.read_u32()? as usize;
    let elem = cursor.read_exact(elem_size)?;
    let mut nested = Cursor::new(elem);
    let packet = decode_packet(&mut nested)?;
    if !nested.remaining().is_empty() {
        return Err(OscError::BadBundle(
            "Bundle element contains trailing bytes".to_owned(),
        ));
    }
    Ok(packet)
}

fn read_osc_args(
    cursor: &mut Cursor<'_>,
    type_tags: impl Iterator<Item = char>,
) -> Result<Vec<OscType>> {
    let mut args = Vec::new();
    let mut stack: Vec<Vec<OscType>> = Vec::new();

    for tag in type_tags {
        match tag {
            '[' => {
                stack.push(args);
                args = Vec::new();
            }
            ']' => {
                let array = OscType::Array(OscArray { content: args });
                let Some(mut parent) = stack.pop() else {
                    return Err(OscError::BadMessage("Encountered ] outside array"));
                };
                parent.push(array);
                args = parent;
            }
            _ => args.push(read_osc_arg(cursor, tag)?),
        }
    }

    if !stack.is_empty() {
        return Err(OscError::BadMessage("Unclosed OSC array type tag"));
    }

    Ok(args)
}

fn read_osc_arg(cursor: &mut Cursor<'_>, tag: char) -> Result<OscType> {
    match tag {
        'f' => cursor.read_f32().map(OscType::Float),
        'd' => cursor.read_f64().map(OscType::Double),
        'i' => cursor.read_i32().map(OscType::Int),
        'h' => cursor.read_i64().map(OscType::Long),
        's' => cursor.read_padded_string().map(OscType::String),
        't' => cursor.read_time_tag().map(OscType::Time),
        'b' => cursor.read_blob().map(OscType::Blob),
        'r' => cursor.read_color().map(OscType::Color),
        'T' => Ok(OscType::Bool(true)),
        'F' => Ok(OscType::Bool(false)),
        'N' => Ok(OscType::Nil),
        'I' => Ok(OscType::Inf),
        'c' => cursor.read_char().map(OscType::Char),
        'm' => cursor.read_midi_message().map(OscType::Midi),
        _ => Err(OscError::BadArg(format!(
            "Type tag \"{tag}\" is not implemented!"
        ))),
    }
}

#[derive(Clone, Copy, Debug)]
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.pos..]
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(OscError::BadPacket("Offset overflow"))?;
        if end > self.bytes.len() {
            return Err(OscError::BadPacket("Incomplete data"));
        }
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_exact(4)?;
        Ok(u32::from_be_bytes(
            bytes.try_into().expect("slice length checked"),
        ))
    }

    fn read_i32(&mut self) -> Result<i32> {
        let bytes = self.read_exact(4)?;
        Ok(i32::from_be_bytes(
            bytes.try_into().expect("slice length checked"),
        ))
    }

    fn read_i64(&mut self) -> Result<i64> {
        let bytes = self.read_exact(8)?;
        Ok(i64::from_be_bytes(
            bytes.try_into().expect("slice length checked"),
        ))
    }

    fn read_f32(&mut self) -> Result<f32> {
        let bytes = self.read_exact(4)?;
        Ok(f32::from_be_bytes(
            bytes.try_into().expect("slice length checked"),
        ))
    }

    fn read_f64(&mut self) -> Result<f64> {
        let bytes = self.read_exact(8)?;
        Ok(f64::from_be_bytes(
            bytes.try_into().expect("slice length checked"),
        ))
    }

    fn read_time_tag(&mut self) -> Result<OscTime> {
        Ok(OscTime {
            seconds: self.read_u32()?,
            fractional: self.read_u32()?,
        })
    }

    fn read_padded_string(&mut self) -> Result<String> {
        let start = self.pos;
        let tail = self.remaining();
        let Some(nul_offset) = tail.iter().position(|byte| *byte == 0) else {
            return Err(OscError::BadString(
                "OSC string is missing a NUL terminator",
            ));
        };
        let nul_pos = start + nul_offset;
        let string_bytes = &self.bytes[start..nul_pos];
        let next = pad(nul_pos + 1);
        if next > self.bytes.len() {
            return Err(OscError::BadString(
                "OSC string padding exceeds packet length",
            ));
        }
        self.pos = next;
        String::from_utf8(string_bytes.to_vec()).map_err(OscError::StringError)
    }

    fn read_blob(&mut self) -> Result<Vec<u8>> {
        let size = self.read_u32()? as usize;
        let start = self.pos;
        let data = self.read_exact(size)?.to_vec();
        let next = pad(start + size);
        if next > self.bytes.len() {
            return Err(OscError::BadArg(
                "Blob padding exceeds packet length".to_owned(),
            ));
        }
        self.pos = next;
        Ok(data)
    }

    fn read_char(&mut self) -> Result<char> {
        let scalar = self.read_u32()?;
        char::from_u32(scalar).ok_or_else(|| {
            OscError::BadArg(format!(
                "Argument value 0x{scalar:08X} is not a Unicode scalar"
            ))
        })
    }

    fn read_midi_message(&mut self) -> Result<OscMidiMessage> {
        let bytes = self.read_exact(4)?;
        Ok(OscMidiMessage {
            port: bytes[0],
            status: bytes[1],
            data1: bytes[2],
            data2: bytes[3],
        })
    }

    fn read_color(&mut self) -> Result<OscColor> {
        let bytes = self.read_exact(4)?;
        Ok(OscColor {
            red: bytes[0],
            green: bytes[1],
            blue: bytes[2],
            alpha: bytes[3],
        })
    }
}

fn pad(pos: usize) -> usize {
    match pos % 4 {
        0 => pos,
        remainder => pos + (4 - remainder),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder;

    #[test]
    fn tcp_decoder_waits_for_incomplete_body() {
        let body = encoder::encode(&OscPacket::Message(OscMessage::from("/a"))).unwrap();
        let mut tcp = Vec::new();
        tcp.extend_from_slice(&(body.len() as u32).to_be_bytes());
        tcp.extend_from_slice(&body[..body.len() - 1]);

        let (remainder, packet) = decode_tcp(&tcp).unwrap();
        assert_eq!(remainder, tcp.as_slice());
        assert!(packet.is_none());
    }

    #[test]
    fn tcp_decoder_waits_for_incomplete_length_prefix() {
        let input = [0, 0, 0];
        let (remainder, packet) = decode_tcp(&input).unwrap();

        assert_eq!(remainder, input.as_slice());
        assert!(packet.is_none());
    }
}
