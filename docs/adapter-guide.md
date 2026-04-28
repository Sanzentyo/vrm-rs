# Adapter Guide

`vrm-rs` does not require Bevy, wgpu, ash, or any specific engine.

Engines integrate by implementing traits from `vrm-adapter`:

- `SceneGraph`: parent/child traversal.
- `TransformAccess`: transform reads/writes.
- `MorphTargetAccess`: expression morph target application.
- `MaterialAccess`: material color and texture transform application.
- `TextureResolver`: texture handle lookup.
- `AnimationSink`: consume runtime events.

For Bevy, the first implementation target is a thin optional adapter that maps `NodeRef` to entities and routes runtime events to components. For custom wgpu/ash engines, keep renderer resources outside `vrm-rs` and use `MaterialRef`/`TextureRef` as stable lookup keys into engine-owned tables.
