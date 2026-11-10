#!/usr/bin/env bash
# Bring up the local-validator stand for the Coffer venue.
#
#   scripts/local-stand/up.sh            # (re)start the validator and build the stand
#   scripts/local-stand/up.sh --no-reset # reuse the running validator, only (re)build pools
#
# Loads: the production Coffer ELF at the mainnet program id, the Titan
# router built from program-template at its declared id, and the live
# CofferPoolConfig (E9K7…) with `protocol_admin` rewritten to the local wallet
# (scripts/local-stand/config.E9K7.local.json — regenerate with
# scripts/local-stand/patch-config.py if your wallet differs).
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$(pwd)
LEDGER="$ROOT/scripts/local-stand/ledger"
COFFER=8iQtGj9mcUfFUGaiCpPy89swC3s8YTC8FhVZWfgeZhwu
ROUTER=T1TANpTeScyeqVzzgNViGDNrkQ6qHz9KrSBS4aNXvGT
ROUTER_SO="$ROOT/program-template/target/deploy/titan_v3_venue_template.so"
CONFIG=E9K7CxPXpAEp49Usgxcxxm9DR8qUFXyvpwLjKrqqtrbY

[ -f "$ROUTER_SO" ] || { echo "missing $ROUTER_SO — run: make build-program"; exit 1; }
WALLET=$(solana address)
python3 scripts/local-stand/patch-config.py "$WALLET"

if [ "${1:-}" != "--no-reset" ]; then
  # Only the stand's own validator (matched by its ledger path): other
  # solana-test-validator instances on this machine are left alone.
  pkill -f "solana-test-validator.*--ledger $LEDGER" || true
  sleep 1
  rm -rf "$LEDGER"
  nohup solana-test-validator --reset --quiet --ledger "$LEDGER" \
    --bpf-program "$COFFER" "$ROOT/programs/$COFFER.so" \
    --bpf-program "$ROUTER" "$ROUTER_SO" \
    --account "$CONFIG" "$ROOT/scripts/local-stand/config.E9K7.local.json" \
    > "$ROOT/scripts/local-stand/validator.log" 2>&1 &
  for i in $(seq 1 60); do
    if curl -s http://127.0.0.1:8899 -X POST -H 'content-type: application/json' \
         -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' | grep -q '"ok"'; then break; fi
    sleep 1
  done
fi
solana --url http://127.0.0.1:8899 airdrop 1000 "$WALLET" >/dev/null || true
cargo run --release --bin local-stand -- setup
