# vrm-rs Bevy web sample

This sample displays a local VRM file, plus an optional VRMA clip, in a browser canvas through `vrm-adapter-bevy`.

```powershell
rustup target add wasm32-unknown-unknown
cargo install wasm-pack basic-http-server
cd examples/wasm/bevy-web
wasm-pack build --target web --release
basic-http-server .
```

Open the printed local URL and select files from `.external-fixtures/` or another local fixture directory. The sample does not commit model binaries.
