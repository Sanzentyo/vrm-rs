# vrm-osc

`vrm-osc` is a dependency-free Open Sound Control (OSC) 1.0 codec intended to live inside the `vrm-rs` workspace.

The crate mirrors the `rosc` packet/value model closely enough for `vrm-rs` users that need OSC or VMC-style transports without taking `rosc`, `nom`, or `byteorder` as runtime dependencies.

## Supported packet model

`vrm-osc` supports:

- `OscPacket::Message`
- `OscPacket::Bundle`
- nested bundle content
- UDP packet decoding
- TCP/stream length-prefixed decoding and encoding
- TCP/stream fragment waiting when the length prefix or packet body is incomplete
- reusable encoder output via `encoder::Output`

## Supported argument types

The codec can decode and encode every `OscType` variant exposed by `rosc` 0.11:

| Type tag | Variant |
|---|---|
| `i` | `OscType::Int(i32)` |
| `f` | `OscType::Float(f32)` |
| `s` | `OscType::String(String)` |
| `b` | `OscType::Blob(Vec<u8>)` |
| `t` | `OscType::Time(OscTime)` |
| `h` | `OscType::Long(i64)` |
| `d` | `OscType::Double(f64)` |
| `c` | `OscType::Char(char)` |
| `r` | `OscType::Color(OscColor)` |
| `m` | `OscType::Midi(OscMidiMessage)` |
| `T` / `F` | `OscType::Bool(bool)` |
| `[` / `]` | `OscType::Array(OscArray)` |
| `N` | `OscType::Nil` |
| `I` | `OscType::Inf` |

## Usage

```rust
use vrm_osc::{decoder, encoder, OscMessage, OscPacket, OscType};

let packet = OscPacket::Message(OscMessage {
    addr: "/VMC/Ext/Bone/Pos".to_owned(),
    args: vec![
        OscType::String("Head".to_owned()),
        OscType::Float(0.0),
        OscType::Float(1.6),
        OscType::Float(0.0),
    ],
});

let bytes = encoder::encode(&packet)?;
let (remainder, decoded) = decoder::decode_udp(&bytes)?;
assert!(remainder.is_empty());
assert_eq!(decoded, packet);
# Ok::<(), vrm_osc::OscError>(())
```

## Integration with `vrm-rs`

The root crate exposes this as an optional feature:

```toml
vrm-rs = { path = ".", features = ["osc"] }
```

Then use:

```rust
use vrm_rs::osc::{decoder, encoder, OscPacket};
```

## Notes

This crate is only an OSC codec. It deliberately does not implement VMC Protocol semantics, VRM humanoid bone mapping, OSC address-pattern matching, or socket transport. Those layers should be built above `vrm-osc` so the codec stays small and reusable.

The implementation is strict in a few places that are useful for production input handling: type tag strings must start with `,`, unclosed arrays are rejected, and TCP packet lengths must match the decoded packet body.
