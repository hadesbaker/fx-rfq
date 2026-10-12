#!/usr/bin/env bash
# Regenerates abi/*.json from the compiled contracts.
#
# The Rust bindings are generated from these artifacts, so the Quote struct and
# every selector come from the contract that was actually compiled. Run this
# after changing anything under contracts/src.
set -euo pipefail
cd "$(dirname "$0")/.."

( cd contracts && forge build )

for c in RfqSettlement MockUSDC MockFixingOracle; do
  src=$(find contracts/out -name "$c.json" | head -1)
  python3 - "$src" "abi/$c.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
json.dump({"abi": d["abi"], "bytecode": d["bytecode"]}, open(sys.argv[2], "w"), indent=1)
PY
  echo "  abi/$c.json"
done
