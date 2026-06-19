# OSC support in `vrm-rs`

This document describes the `vrm-osc` crate added for OSC/VMC-style motion streaming support.

## Goal

`vrm-osc` provides a local, dependency-free OSC 1.0 codec used by the optional `vrm-vmc` VMC Protocol layer without pulling `rosc` into the runtime dependency graph. It is deliberately a generic OSC codec rather than a VMC-specific parser.

The intended stack is:

```text
UDP/TCP socket layer          optional, outside this crate
OSC packet codec             crates/vrm-osc
VMC Protocol message mapping  crates/vrm-vmc
VRM humanoid/runtime apply    vrm-core / vrm-runtime / vrm-adapter
```

## Repository changes

Add the crate to the workspace:

```toml
[workspace]
members = [
    ".",
    "crates/vrm-adapter",
    "crates/vrm-adapter-bevy",
    "crates/vrm-core",
    "crates/vrm-io",
    "crates/vrm-osc",
    "crates/vrm-protocol",
    "crates/vrm-runtime",
    "crates/vrm-sans-io",
]
```

Expose it as an optional root feature:

```toml
[features]
default = []
osc = ["dep:vrm-osc"]

[dependencies]
vrm-osc = { path = "crates/vrm-osc", optional = true }
```

Re-export it from the root facade:

```rust
#[cfg(feature = "osc")]
pub use vrm_osc as osc;
```

## API overview

Decode one UDP packet:

```rust
let (remainder, packet) = vrm_osc::decoder::decode_udp(bytes)?;
```

Decode one TCP/stream packet with OSC 1.0 length prefix:

```rust
let (remainder, packet) = vrm_osc::decoder::decode_tcp(bytes)?;
```

`packet` is `None` when the slice does not yet contain the complete 32-bit
length prefix or the complete packet body, so callers can keep buffering stream
data without treating normal fragmentation as malformed input.

Decode all complete TCP/stream packets from a slice:

```rust
let (remainder, packets) = vrm_osc::decoder::decode_tcp_vec(bytes)?;
```

Encode into a fresh buffer:

```rust
let bytes = vrm_osc::encoder::encode(&packet)?;
let tcp_bytes = vrm_osc::encoder::encode_tcp(&packet)?;
```

Encode into a reusable buffer:

```rust
let mut bytes = Vec::new();
vrm_osc::encoder::encode_into(&packet, &mut bytes)?;
```

## Supported values

The supported value set mirrors `rosc` 0.11:

- `OscType::Int(i32)` / `i`
- `OscType::Float(f32)` / `f`
- `OscType::String(String)` / `s`
- `OscType::Blob(Vec<u8>)` / `b`
- `OscType::Time(OscTime)` / `t`
- `OscType::Long(i64)` / `h`
- `OscType::Double(f64)` / `d`
- `OscType::Char(char)` / `c`
- `OscType::Color(OscColor)` / `r`
- `OscType::Midi(OscMidiMessage)` / `m`
- `OscType::Bool(bool)` / `T` or `F`
- `OscType::Array(OscArray)` / `[` and `]`
- `OscType::Nil` / `N`
- `OscType::Inf` / `I`

## Compatibility notes

`vrm-osc` is intended to be wire-compatible with `rosc` for encoding and decoding the supported packet/value model. A few validation choices are intentionally stricter:

- Message type-tag strings must start with `,`.
- `]` outside an array is an error.
- Unclosed array type tags are an error.
- TCP body lengths must match the packet body consumed by the decoder.

These checks make malformed network input fail early and avoid silently accepting ambiguous packets.

## Testing plan

The crate includes integration tests for:

- round-tripping every supported argument type;
- nested arrays;
- nested bundles;
- TCP `encode_tcp` / `decode_tcp_vec`;
- TCP partial-prefix and partial-body waiting;
- OSC string padding;
- unsupported type tag errors.

When applying this to the repository, run:

```bash
cargo test -p vrm-osc
cargo test --features osc
```

## VMC layer

The optional `vrm-vmc` crate depends on `vrm-osc` and translates OSC packets into typed VMC events, then applies them through `VmcRuntimeSink`. It covers the core VMC 3.1 Marionette/Performer message families, recursive bundle traversal, `/VMC/Thru/*` passthrough, and strict or invalid-message-skipping parse policy. Socket ownership, reconnect behavior, authentication, rate limits, and jitter buffers remain application policy rather than part of the codec crate.

Keeping OSC generic here prevents VMC-specific assumptions from leaking into the lower-level packet model.
