# vrm-rs wgpu web sample

This sample displays a VRM file plus two switchable VRMA clips in a browser canvas through `vrm-adapter-wgpu`. The animation switcher is rendered as a small `egui` overlay on top of the wgpu canvas. Switching motions uses the generic `vrm-runtime` animation mixer with a 0.35-second linear crossfade instead of replacing the pose instantly.

Published sample: <https://sanzentyo.github.io/vrm-rs/wasm-example/>

```powershell
rustup target add wasm32-unknown-unknown
cargo install wasm-pack basic-http-server
cd examples/wasm/wgpu-web
wasm-pack build --target web --release
cd ../../..
basic-http-server .
```

Open the printed local URL with `/examples/wasm/wgpu-web/` appended. The page defaults to the official `Seed-san.vrm` sample from `vrm-c/vrm-specification`, plus pinned `pixiv/three-vrm` sample clips: `idle_loop.vrma` as Motion A and `test.vrma` as Motion B. The file inputs can override any source.

## External assets and licensing

The repository and the published GitHub Pages site do not contain VRM or VRMA binaries. The browser fetches the initial URLs directly from their pinned upstream GitHub revisions:

- `Seed-san.vrm`: authored by VirtualCast, Inc. Its embedded VRM 1.0 metadata permits redistribution and modification redistribution; the avatar remains subject to its embedded VRM usage conditions.
- `idle_loop.vrma` and `test.vrma`: fetched from the MIT-licensed `pixiv/three-vrm` repository. Their standalone asset provenance is not stated separately, so they are external-only defaults and are not redistributed by `vrm-rs` or its Pages deployment.

When replacing any URL or choosing a local file, the user is responsible for complying with that asset's author, license, and VRM metadata permissions.
