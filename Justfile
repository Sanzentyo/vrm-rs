set shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

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
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-background "{{ background }}" --render-mtoon-light-accumulation "{{ light_accumulation }}" --render-fixture Seed-san.vrm --render-fixture VRM1_Constraint_Twist_Sample.vrm --render-fixture .external-fixtures/official/vrm-specification/samples/VRMC_materials_mtoon_UV_Animation_Test/VRMC_materials_mtoon_UV_Animation_Test.vrm --render-fixture .external-fixtures/official/vrm-specification/samples/VRMC_vrm_expressions_isBinary_Overridden/VRMC_vrm_expressions_isBinary_Overridden.vrm --render-fixture .external-fixtures/official/vrm-specification/samples/VRMC_vrm_expressions_isBinary_Overrides/VRMC_vrm_expressions_isBinary_Overrides.vrm --render-fixture .external-fixtures/official/UniVRM/AliciaSolid_vrm-0.51.vrm

# Regenerate a time-advanced MToon UV animation parity artifact.
render-parity-uv-animation three_vrm_root="D:/git/three-vrm" time="1.0" background="opaque-black" light_accumulation="tuned":
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{ three_vrm_root }}" --render-parity-dir .external-fixtures/render-parity-uv-animation --render-mtoon-time {{ time }} --render-background "{{ background }}" --render-mtoon-light-accumulation "{{ light_accumulation }}" --render-fixture .external-fixtures/official/vrm-specification/samples/VRMC_materials_mtoon_UV_Animation_Test/VRMC_materials_mtoon_UV_Animation_Test.vrm

# Prepare external inputs and regenerate the default render parity artifact set from scratch.
render-parity-full:
    cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity

# Run the coverage gate used by local CI.
coverage:
    cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70
