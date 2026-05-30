set shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

light_swatch_names := "direct-base,forced-shade,ambient-ao-ignored,parametric-rim,matcap-rim,mixed-rim,toon-ramp-lit-normal,toon-ramp-shade-normal,emissive-factor,emissive-texture-strength"

default:
    @just --list

# Run the local CI-equivalent gate.
ci:
    cargo +nightly -Zscript tools/ci/local-ci.rs

# Download external fixtures, regenerate goldens, and run ignored parity tests locally.
ci-external:
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures

# Regenerate the default render parity artifacts using existing fixtures and three-vrm checkout.
render-parity three_vrm_root="D:/git/three-vrm" background="opaque-black" light_accumulation="tuned":
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-background "{{ background }}" --render-mtoon-light-accumulation "{{ light_accumulation }}"

# Regenerate render parity artifacts for the current local official VRM sample set.
render-parity-samples three_vrm_root="D:/git/three-vrm" background="opaque-black" light_accumulation="tuned":
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-background "{{ background }}" --render-mtoon-light-accumulation "{{ light_accumulation }}" --render-psnr-metric rgb-visible --render-fail-under 32 --render-fixture Seed-san.vrm --render-fixture VRM1_Constraint_Twist_Sample.vrm --render-fixture .external-fixtures/official/vrm-specification/samples/VRMC_materials_mtoon_UV_Animation_Test/VRMC_materials_mtoon_UV_Animation_Test.vrm --render-fixture .external-fixtures/official/vrm-specification/samples/VRMC_vrm_expressions_isBinary_Overridden/VRMC_vrm_expressions_isBinary_Overridden.vrm --render-fixture .external-fixtures/official/vrm-specification/samples/VRMC_vrm_expressions_isBinary_Overrides/VRMC_vrm_expressions_isBinary_Overrides.vrm --render-fixture .external-fixtures/official/UniVRM/AliciaSolid_vrm-0.51.vrm

# Regenerate transparent-background artifacts for real external fixtures.
render-parity-real-transparent three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-real-transparent --render-background transparent --render-alpha-mismatch-tolerance 64 --render-psnr-metric rgb-all --render-fail-under 32 --render-mtoon-light-accumulation tuned --render-fixture Seed-san.vrm --render-fixture VRM1_Constraint_Twist_Sample.vrm --render-fixture .external-fixtures/official/vrm-specification/samples/VRMC_materials_mtoon_UV_Animation_Test/VRMC_materials_mtoon_UV_Animation_Test.vrm --render-fixture .external-fixtures/official/vrm-specification/samples/VRMC_vrm_expressions_isBinary_Overridden/VRMC_vrm_expressions_isBinary_Overridden.vrm --render-fixture .external-fixtures/official/vrm-specification/samples/VRMC_vrm_expressions_isBinary_Overrides/VRMC_vrm_expressions_isBinary_Overrides.vrm --render-fixture .external-fixtures/official/UniVRM/AliciaSolid_vrm-0.51.vrm

# Regenerate focused artifacts for real official MToon normal-map fixtures whose primitives omit glTF TANGENT.
render-parity-real-normal-maps three_vrm_root="D:/git/three-vrm" background="opaque-black":
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-real-normal-maps --render-background "{{ background }}" --render-alpha-mismatch-tolerance 0 --render-psnr-metric rgb-visible --render-fail-under 32.5 --render-mtoon-light-accumulation tuned --render-fixture Seed-san.vrm --render-fixture VRM1_Constraint_Twist_Sample.vrm

# Re-measure the reference-shaped MToon light/color accumulator without tuned exposure.
render-parity-light-three-vrm three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-three-vrm-light --render-background opaque-black --render-mtoon-light-accumulation three-vrm --render-fixture Seed-san.vrm --render-fixture VRM1_Constraint_Twist_Sample.vrm

# Diagnostic: disable outlines in three-vrm, wgpu, and Bevy to isolate material/pose deltas from outline expansion.
render-parity-outline-off three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-outline-off --render-background opaque-black --render-alpha-mismatch-tolerance 0 --render-psnr-metric rgb-visible --render-fail-under 32 --render-disable-outlines --render-fixture Seed-san.vrm --render-fixture VRM1_Constraint_Twist_Sample.vrm

# Generate and render a source-like MToon light/color accumulation fixture.
render-parity-mtoon-light-generated three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-mtoon-light-fixture.rs
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-mtoon-light-generated --render-background transparent --render-alpha-mismatch-tolerance 512 --render-psnr-metric rgb-interior1px --render-fail-under 50 --render-mtoon-light-accumulation three-vrm --render-fixture .external-fixtures/generated/mtoon-light.vrm.gltf
    cargo +nightly -Zscript tools/render-parity/compare-swatch-colors.rs --expected .external-fixtures/render-parity-mtoon-light-generated/three-vrm/mtoon-light_vrm.frame000.rgba.json --actual .external-fixtures/render-parity-mtoon-light-generated/wgpu/mtoon-light_vrm.frame000.rgba.json --names "{{ light_swatch_names }}" --fail-under 50 --max-channel-delta 1 --json-out .external-fixtures/render-parity-mtoon-light-generated/reports/mtoon-light_vrm.wgpu.swatches.json
    cargo +nightly -Zscript tools/render-parity/compare-swatch-colors.rs --expected .external-fixtures/render-parity-mtoon-light-generated/three-vrm/mtoon-light_vrm.frame000.rgba.json --actual .external-fixtures/render-parity-mtoon-light-generated/bevy/mtoon-light_vrm.frame000.rgba.json --names "{{ light_swatch_names }}" --fail-under 47 --max-channel-delta 2 --json-out .external-fixtures/render-parity-mtoon-light-generated/reports/mtoon-light_vrm.bevy.swatches.json

# Generate and render the MToon light/color fixture with ambient disabled on both sides.
render-parity-mtoon-light-direct-generated three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-mtoon-light-fixture.rs
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-mtoon-light-direct-generated --render-background transparent --render-alpha-mismatch-tolerance 512 --render-psnr-metric rgb-interior1px --render-fail-under 50 --render-mtoon-light-accumulation three-vrm --render-pbr-ambient 0 --render-three-vrm-ambient-intensity 0 --render-fixture .external-fixtures/generated/mtoon-light.vrm.gltf

# Generate and render the MToon light/color fixture with directional light disabled on both sides.
render-parity-mtoon-light-ambient-generated three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-mtoon-light-fixture.rs
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-mtoon-light-ambient-generated --render-background transparent --render-alpha-mismatch-tolerance 512 --render-psnr-metric rgb-interior1px --render-fail-under 50 --render-mtoon-light-accumulation three-vrm --render-direct-light-scale 0 --render-three-vrm-directional-intensity 0 --render-fixture .external-fixtures/generated/mtoon-light.vrm.gltf

# Inspect local fixtures for MToon material features that should be covered by render parity.
inspect-mtoon-fixtures root=".external-fixtures/official":
    cargo +nightly -Zscript tools/render-parity/inspect-mtoon-fixtures.rs -- --root "{{ root }}"

# Generate and render a source-like MToon texture-slot fixture.
render-parity-mtoon-textures-generated three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-mtoon-texture-fixture.rs
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-mtoon-textures-generated --render-background transparent --render-alpha-mismatch-tolerance 512 --render-psnr-metric rgb-interior1px --render-fail-under 50 --render-mtoon-light-accumulation three-vrm --render-mtoon-time 1.0 --render-fixture .external-fixtures/generated/mtoon-texture-slots.vrm.gltf

# Generate and render an opt-in MToon normal-map fixture that exercises three-vrm's tangentless fallback.
render-parity-mtoon-normal-generated three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-mtoon-texture-fixture.rs --include-normal --out .external-fixtures/generated/mtoon-normal-texture.vrm.gltf
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-mtoon-normal-generated --render-background transparent --render-alpha-mismatch-tolerance 0 --render-psnr-metric rgb-interior1px --render-fail-under 45 --render-mtoon-light-accumulation three-vrm --render-mtoon-time 1.0 --render-fixture .external-fixtures/generated/mtoon-normal-texture.vrm.gltf

# Generate and render a source-like MToon screen-coordinate outline fixture.
render-parity-screen-outline-generated three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-screen-outline-fixture.rs
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-screen-outline-generated --render-background transparent --render-alpha-mismatch-tolerance 256 --render-psnr-metric rgb-opaque --render-fail-under 50 --render-mtoon-light-accumulation three-vrm --render-fixture .external-fixtures/generated/screen-outline.vrm.gltf

# Generate and render a source-like expression morph fixture.
render-parity-morph-expression-generated three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-morph-expression-fixture.rs
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-morph-expression-generated --render-background transparent --render-alpha-mismatch-tolerance 8 --render-psnr-metric rgb-interior1px --render-fail-under 50 --render-mtoon-light-accumulation three-vrm --render-expression happy=1.0 --render-fixture .external-fixtures/generated/morph-expression.vrm.gltf

# Regenerate a time-advanced MToon UV animation parity artifact.
render-parity-uv-animation three_vrm_root="D:/git/three-vrm" time="1.0" background="opaque-black" light_accumulation="tuned":
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-uv-animation --render-mtoon-time {{ time }} --render-background "{{ background }}" --render-mtoon-light-accumulation "{{ light_accumulation }}" --render-fixture .external-fixtures/official/vrm-specification/samples/VRMC_materials_mtoon_UV_Animation_Test/VRMC_materials_mtoon_UV_Animation_Test.vrm

# Generate the source-like local transparent MToon fixture.
generate-transparent-fixture:
    cargo +nightly -Zscript tools/render-parity/generate-transparent-fixture.rs

# Regenerate transparent-background alpha/blend parity artifacts for the generated fixture.
render-parity-transparent-generated three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-transparent-fixture.rs
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-transparent-generated --render-background transparent --render-alpha-mismatch-tolerance 0 --render-psnr-metric rgb-visible --render-fail-under 49 --render-max-selected-channel-delta 2 --render-max-alpha-delta 0 --render-fixture .external-fixtures/generated/transparent-blend.vrm.gltf

# Regenerate high-contrast transparent material ordering artifacts for Bevy/wgpu parity debugging.
render-parity-transparent-high-contrast three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-transparent-fixture.rs --palette high-contrast --out .external-fixtures/generated/transparent-high-contrast.vrm.gltf
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-transparent-high-contrast --render-background transparent --render-alpha-mismatch-tolerance 0 --render-psnr-metric rgb-visible --render-fail-under 51 --render-max-selected-channel-delta 2 --render-max-alpha-delta 0 --render-fixture .external-fixtures/generated/transparent-high-contrast.vrm.gltf

# Regenerate broader transparent material artifacts with texture alpha and mixed render queues.
render-parity-transparent-broad three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-transparent-fixture.rs --case broad --palette high-contrast --out .external-fixtures/generated/transparent-broad.vrm.gltf
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-transparent-broad --render-background transparent --render-alpha-mismatch-tolerance 0 --render-alpha-channel-tolerance 1 --render-psnr-metric rgb-visible --render-fail-under 48 --render-max-selected-channel-delta 4 --render-max-alpha-delta 1 --render-fixture .external-fixtures/generated/transparent-broad.vrm.gltf

# Regenerate alpha-mode transparent material artifacts for OPAQUE/MASK/BLEND cutoff parity.
render-parity-transparent-alpha-modes three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-transparent-alpha-modes-fixture.rs
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-transparent-alpha-modes --render-background transparent --render-alpha-mismatch-tolerance 0 --render-alpha-channel-tolerance 0 --render-psnr-metric rgb-visible --render-fail-under 47 --render-max-selected-channel-delta 2 --render-max-alpha-delta 0 --render-fixture .external-fixtures/generated/transparent-alpha-modes.vrm.gltf

# Regenerate transparent depth-sort artifacts for same-render-order BLEND layers.
render-parity-transparent-depth-stack three_vrm_root="D:/git/three-vrm":
    cargo +nightly -Zscript tools/render-parity/generate-transparent-depth-fixture.rs
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-transparent-depth-stack --render-background transparent --render-alpha-mismatch-tolerance 0 --render-alpha-channel-tolerance 1 --render-psnr-metric rgb-visible --render-fail-under 49 --render-max-selected-channel-delta 2 --render-max-alpha-delta 1 --render-fixture .external-fixtures/generated/transparent-depth-stack.vrm.gltf

# Prepare external inputs and regenerate the default render parity artifact set from scratch.
render-parity-full:
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity

# Run the coverage gate used by local CI.
coverage:
    cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70
