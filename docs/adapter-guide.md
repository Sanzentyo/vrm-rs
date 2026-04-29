# Adapter Guide

`vrm-rs` does not require Bevy, wgpu, ash, or any specific engine.

Engines integrate by implementing traits from `vrm-adapter`:

- `SceneGraph`: parent/child traversal.
- `TransformAccess`: transform reads/writes.
- `MorphTargetAccess`: expression morph target application.
- `MaterialAccess`: material color, texture transform, and HDR emissive intensity application.
- `MtoonPipelineAccess`: MToon base/outline pipeline pass selection.
- `TextureResolver`: texture handle lookup.
- `AnimationSink`: consume runtime events.

Engines that want humanoid pose parity can capture a `HumanoidPoseRig` from their scene graph. It provides raw absolute/rest-relative pose access, normalized absolute/rest-relative pose storage, and normalized-to-raw writeback through transform traits.

Engines that want a single orchestration point can use `VrmRuntimeDriver`. It applies animation frames, runtime expression events, node constraints, spring bone, first-person visibility, one-shot VRM0 root orientation compatibility, MToon pipeline hints, and effective emissive strength writeback through the traits above.

For Bevy, the first implementation target is a thin optional adapter that maps `NodeRef` to entities and routes runtime events to components. For custom wgpu/ash engines, keep renderer resources outside `vrm-rs` and use `MaterialRef`/`TextureRef` as stable lookup keys into engine-owned tables.
