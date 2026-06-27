# Quay × Titan integration — developer entry points.
#
#   make build-program   run anchor build for the route program
#   make check-structure lib tests + scorecard assertion + enum parity (no RPC)
#   make test-venue      the Quay venue suite (off-chain + on-chain route)
#   make scorecard       print the integration scorecard only
#   make dump-programs   fetch the Quay program binary the sim tests load
#
# Each phase reports one of:
#   ok       ran and passed
#   skipped  could not run — missing SOLANA_RPC_URL (and/or the program dump / a
#            built on-chain program); nothing was actually exercised
#   FAILED   ran and failed
#
# To actually run the RPC-gated tests:
#   export SOLANA_RPC_URL=https://...   &&   make build-program   &&   make dump-programs
#   make test-venue

QUAY := QUayE6nexQWYNZAEqfN8FxoNwQDSu3CAzT2qq9J1ArG
PROGRAMS := $(QUAY)

DUMP_URL := $(if $(SOLANA_RPC_URL),$(SOLANA_RPC_URL),m)

PROGRAM := --manifest-path program-template/Cargo.toml
RELEASE_PROFILE := --release
# Used only for the construction allocation guard. Quote-speed runs in release.
ASSERT_PROFILE := --profile release-debug
SCORECARD = cargo test --quiet $(RELEASE_PROFILE) --test scorecard -- --nocapture 2>/dev/null | sed -n '/^====/,/^====/p'

.PHONY: build-program check-structure test-venue scorecard dump-programs \
        _unit-phase _venue-phase

# --- always-on checks (no RPC): unit tests, scorecard assertion, enum parity ---
_unit-phase:
	@mkdir -p target
	@printf '\n================ Quay structural checks =====================\n\n'
	@printf '  %-24s  %-8s  %s\n' 'Check' 'Status' 'Detail'
	@printf '  %-24s  %-8s  %s\n' '------------------------' '--------' '----------------------------------------'
	@log=target/log-unit.txt; \
		if cargo test --quiet $(RELEASE_PROFILE) --lib --test scorecard >$$log 2>&1 \
			&& cargo test --quiet $(PROGRAM) --release --lib --test venue_parity >>$$log 2>&1; \
		then st=ok; dt='lib tests + scorecard + enum parity'; \
		else st=FAILED; dt='see log below'; fi; \
		printf '  %-24s  %-8s  %s\n' 'Unit + structure' "$$st" "$$dt"; \
		if [ $$st = FAILED ]; then echo; cat $$log; exit 1; fi

# --- the Quay venue suite: needs RPC (+ program dump / anchor build for sims) ---
# Off-chain suite runs single-threaded so the `quoting_speed` benchmark (the
# quote path is ~1µs of simulator work) isn't starved of CPU by the concurrent
# LiteSVM simulation tests.
_venue-phase:
	@mkdir -p target
	@printf '\n================ Quay venue checks ==========================\n\n'
	@printf '  %-24s  %-8s  %s\n' 'Check' 'Status' 'Detail'
	@printf '  %-24s  %-8s  %s\n' '------------------------' '--------' '----------------------------------------'
	@log=target/log-venue-off.txt; \
		cargo test --quiet $(RELEASE_PROFILE) --test quay -- --skip construction --test-threads=1 --nocapture >$$log 2>&1; rc1=$$?; \
		cargo test --quiet $(ASSERT_PROFILE) --test quay -- construction --nocapture >>$$log 2>&1; rc2=$$?; \
		cargo test --quiet $(RELEASE_PROFILE) --test quay_creation -- --nocapture >>$$log 2>&1; rc3=$$?; \
		if [ $$rc1 -ne 0 ] || [ $$rc2 -ne 0 ] || [ $$rc3 -ne 0 ]; then st=FAILED; dt='see log below'; \
		elif grep -q 'SKIP' $$log; then st=skipped; dt='set SOLANA_RPC_URL'; \
		else st=ok; dt='off-chain quote + creation passed'; fi; \
		printf '  %-24s  %-8s  %s\n' 'Off-chain' "$$st" "$$dt"; \
		if [ $$st = FAILED ]; then echo; cat $$log; exit 1; fi
	@log=target/log-venue-prog.txt; \
		cargo test --quiet $(PROGRAM) --release --test quay_route -- --nocapture >$$log 2>&1; rc=$$?; \
		if [ $$rc -ne 0 ]; then st=FAILED; dt='see log below'; \
		elif grep -q 'SKIP' $$log; then st=skipped; dt='needs fresh anchor build + RPC + dump'; \
		else st=ok; dt='on-chain route passed'; fi; \
		printf '  %-24s  %-8s  %s\n' 'On-chain program' "$$st" "$$dt"; \
		if [ $$st = FAILED ]; then echo; cat $$log; exit 1; fi

# --- public targets -----------------------------------------------------------
build-program:
	@cd program-template && anchor build

check-structure: _unit-phase

test-venue: _venue-phase
	@echo
	@$(SCORECARD)

scorecard:
	@$(SCORECARD)

# Dump the Quay program binary into programs/ if it isn't already there.
# Requires the `solana` CLI on PATH.
dump-programs:
	@mkdir -p programs
	@for p in $(PROGRAMS); do \
		if [ -f programs/$$p.so ]; then \
			echo "have programs/$$p.so"; \
		else \
			echo "dumping $$p from $(DUMP_URL)"; \
			solana program dump -u $(DUMP_URL) $$p programs/$$p.so; \
		fi; \
	done
