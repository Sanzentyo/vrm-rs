set shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
  @just --list

# Run the local CI-equivalent gate.
ci:
  cargo +nightly -Zscript tools/ci/local-ci.rs

# Download external fixtures, regenerate goldens, and run ignored parity tests locally.
ci-external:
  cargo +nightly -Zscript tools/ci/local-ci.rs -- --external-fixtures

# Regenerate Seed-san render parity artifacts using an existing fixture and three-vrm checkout.
render-parity three_vrm_root="D:/git/three-vrm":
  cargo +nightly -Zscript tools/ci/local-ci.rs -- --skip-core --skip-coverage --skip-download --skip-three-vrm-build --skip-playwright-install --render-parity --three-vrm-root "{{three_vrm_root}}"

# Prepare external inputs and regenerate Seed-san render parity artifacts from scratch.
render-parity-full:
  cargo +nightly -Zscript tools/ci/local-ci.rs -- --render-parity

# Run the coverage gate used by local CI.
coverage:
  cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70
