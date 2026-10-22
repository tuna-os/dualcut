#!/bin/sh
# Run the real document module's tests without the native media/UI stack.
set -eu
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
harness=$(mktemp -d "${TMPDIR:-/tmp}/dualcut-document.XXXXXX")
trap 'rm -rf "$harness"' EXIT HUP INT TERM
mkdir "$harness/src"
ln -s "$repo/engine/src/document.rs" "$harness/src/document.rs"
ln -s "$repo/engine/src/document" "$harness/src/document"
ln -s "$repo/engine/examples" "$harness/examples"
cat > "$harness/src/lib.rs" <<'RUST'
pub mod document;
RUST
cat > "$harness/Cargo.toml" <<'TOML'
[package]
name = "dualcut-document-tests"
version = "0.0.0"
edition = "2024"

[dependencies]
anyhow = "1.0.103"
serde = { version = "=1.0.219", features = ["derive"] }
serde_json = "1.0.120"
TOML
cargo test --manifest-path "$harness/Cargo.toml" "$@"
