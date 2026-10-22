#!/usr/bin/env bash
# Run from any directory; keep reports outside Cargo's build tree.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../engine"
export CARGO_LLVM_COV_TARGET_DIR="$PWD/target/llvm-cov-target"
report_dir="$PWD/../coverage"
mkdir -p "$report_dir"
rm -f "$report_dir"/{lcov.info,summary.txt,metadata.txt,raw-profiles.tar.gz}
cargo llvm-cov clean --workspace
{
  git rev-parse HEAD
  rustc -Vv
  cargo llvm-cov --version
  echo 'Scope: cargo test --locked --all-features (no doctest instrumentation or smoke coverage)'
} > "$report_dir/metadata.txt"

# Preserve the test failure status while still exporting diagnostic coverage.
status=0
cargo llvm-cov --locked --all-features --no-report || status=$?
report_status=0
cargo llvm-cov report --all-features --lcov --output-path "$report_dir/lcov.info" || report_status=$?
cargo llvm-cov report --all-features > "$report_dir/summary.txt" || report_status=$?
# Raw profiles and merged data are useful even when a test or report fails.
if [ -d "$CARGO_LLVM_COV_TARGET_DIR" ]; then
  (cd "$CARGO_LLVM_COV_TARGET_DIR" && find . -type f \( -name '*.profraw' -o -name '*.profdata' \) -print0 \
    | tar --null -czf "$report_dir/raw-profiles.tar.gz" -T -)
fi
if [ "$status" -ne 0 ]; then
  exit "$status"
fi
exit "$report_status"
