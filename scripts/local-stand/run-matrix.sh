#!/usr/bin/env bash
# Run the whole local-stand matrix, in the order the tests assume (the shared
# suite and the route simulation read the static pools before the real
# transactions mutate them). Logs go to scripts/local-stand/logs/.
set -uo pipefail
cd "$(dirname "$0")/../.."
export SOLANA_RPC_URL=${SOLANA_RPC_URL:-http://127.0.0.1:8899}
LOG=scripts/local-stand/logs; mkdir -p "$LOG"
[ -f scripts/local-stand/stand.json ] || { echo "no stand.json — run scripts/local-stand/up.sh"; exit 1; }
rc=0
run() { # name, then the command
  local name=$1; shift
  echo "== $name"; "$@" > "$LOG/$name.log" 2>&1; local r=$?
  grep -E "^(  |==|test result|SKIP)" "$LOG/$name.log" | tail -60
  [ $r -eq 0 ] || { echo "   -> FAILED (see $LOG/$name.log)"; rc=1; }
}
run 1-shared-suite        cargo test --release --test local_stand_matrix -- --nocapture --test-threads=1 shared_suite
run 1b-construction-noalloc cargo test --profile release-debug --test local_stand_matrix -- --nocapture --test-threads=1 shared_suite_construction_no_alloc
run 2-route-simulation    cargo test --manifest-path program-template/Cargo.toml --release --test local_stand_route -- --nocapture
run 3-real-routed-txs     cargo test --release --test local_stand_matrix -- --nocapture --test-threads=1 real_routed
run 4-dynamic-sequences   cargo test --release --test local_stand_matrix -- --nocapture --test-threads=1 dynamic_
run 5-range-manager       cargo test --release --test local_stand_range_manager -- --nocapture --test-threads=1
exit $rc
