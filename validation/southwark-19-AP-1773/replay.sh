#!/usr/bin/env bash
# Offline existing Rust pipeline; source PDF/runtime/cache and reports remain local.
set -euo pipefail
repo_dir=$(cd "$(dirname "$0")/../.." && pwd)
case_dir="$repo_dir/validation/southwark-19-AP-1773"
pdf_path=${1:-"$repo_dir/data/raw/southwark_s106_sample.pdf"}
out_dir="$repo_dir/data/processed/validation/southwark-19-AP-1773"
replay_dir=$(mktemp -d)
trap 'rm -rf "$replay_dir"' EXIT
cd "$repo_dir"
python3 - "$case_dir" "$pdf_path" <<'PY'
import hashlib,json,pathlib,sys
r=pathlib.Path(sys.argv[1]);p=json.loads((r/'provenance.json').read_text())
assert hashlib.sha256(pathlib.Path(sys.argv[2]).read_bytes()).hexdigest()==p['sha256'], 'source PDF bytes changed'
receipt=json.loads((r/'fixture-receipt.json').read_text())
assert hashlib.sha256(pathlib.Path(receipt['path']).read_bytes()).hexdigest()==receipt['sha256'], 'model fixture bytes changed'
PY
cp "$pdf_path" "$replay_dir/southwark_s106_sample.pdf"
mkdir -p "$out_dir"
set +e
cargo run --offline --release -- audit "$replay_dir/southwark_s106_sample.pdf" --llm-mode replay --out "$out_dir/rust-audit.json" > "$out_dir/rust-audit.txt" 2> "$out_dir/run.log"
audit_status=$?
set -e
if [ "$audit_status" -ne 1 ]; then
  cat "$out_dir/run.log" >&2
  echo "Unexpected audit exit: $audit_status (expected 1)" >&2
  exit 1
fi
python3 scripts/compare-southwark-validation.py --audit "$out_dir/rust-audit.json" --out "$out_dir/comparison.json"
python3 - "$case_dir/expected-summary.json" "$out_dir/comparison.json" <<'PY'
import json,sys
actual=json.load(open(sys.argv[2]))
summary={k:v for k,v in actual.items() if k not in ['rows','locator_checks']}
assert json.load(open(sys.argv[1]))==summary, 'comparison summary changed; inspect full local comparison'
PY
cargo test --offline --test validation_mutations
python3 -m unittest discover -s tests/python -v
