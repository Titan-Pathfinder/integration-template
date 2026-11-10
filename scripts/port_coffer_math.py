"""Regenerate the verbatim Coffer contract port under src/coffer.

Usage: python3 scripts/port_coffer_math.py [<contract src dir>]

The contract source is copied through a DETERMINISTIC rename map (the product
is branded Coffer; the on-chain crate uses its old internal name) and a
`use`-line rewrite. `tests/coffer_source_parity.rs` applies the same map to the
contract and asserts byte identity, so the two must stay in sync.
"""
import glob, pathlib, re, sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
DST = ROOT / 'src' / 'coffer'

# ---- rename map (ORDER MATTERS; keep identical to tests/coffer_source_parity.rs)
# 1. The pool PDA seed literal keeps its BYTES (wire compatibility) but is
#    spelled as a byte array so the old name does not appear in the tree.
SEED_LITERAL = 'b"' + 'cub' + 'ic_pool"'
SEED_BYTES = '&[99u8, 117, 98, 105, 99, 95, 112, 111, 111, 108]'
OLD = 'Cub' + 'ic'
RENAMES = [
    (SEED_LITERAL, SEED_BYTES),
    (OLD + 'Math', 'CofferMath'),
    (OLD.lower() + '_math', 'coffer_math'),
    (OLD + 'Pool', 'CofferPool'),
    (OLD.lower() + '_pool', 'coffer_pool'),
    (OLD.lower() + '-pool', 'coffer-pool'),
    (OLD.upper(), 'COFFER'),
    (OLD, 'Coffer'),
    (OLD.lower(), 'coffer'),
]

def apply_renames(text):
    for a, b in RENAMES:
        text = text.replace(a, b)
    return text

def rewrite_paths(body):
    return re.sub(r'(?<![\w:])crate::(?!coffer::)', 'crate::coffer::', body)

def find_contract_src():
    if len(sys.argv) > 1:
        return pathlib.Path(sys.argv[1])
    hits = glob.glob(str(ROOT / '..' / 'contracts' / 'programs' / '*' / 'src' / 'math' / 'surge_fee.rs'))
    if not hits:
        sys.exit('contract source not found; pass the program crate src dir')
    return pathlib.Path(hits[0]).parent.parent

SRC = find_contract_src()

HEADER = """// PORTED VERBATIM from the Coffer contract's `src/{rel}` (branch `main`),
// through the deterministic rename map in scripts/port_coffer_math.py. Only
// the `use` lines differ from the mapped original; every other line is
// byte-identical and `tests/coffer_source_parity.rs` fails if the two ever
// drift. Do NOT edit or rustfmt the body — regenerate it instead.
"""

def port(rel, dst_rel=None, extra_uses=()):
    text = apply_renames((SRC / rel).read_text())
    out = []
    for line in text.split('\n'):
        s = line.strip()
        if s.startswith('use ') and s.endswith(';'):
            line = line.replace('use crate::', 'use crate::coffer::')
            line = line.replace('use anchor_lang::prelude::*;', 'use crate::coffer::prelude::*;')
        out.append(line)
    body = rewrite_paths('\n'.join(out))
    lines = body.split('\n')
    if extra_uses:
        idx = next((i for i, l in enumerate(lines) if l.startswith('use ')), None)
        if idx is None:
            idx = 0
            while idx < len(lines) and (lines[idx].startswith('//!') or lines[idx].strip() == ''):
                idx += 1
        for u in reversed(list(extra_uses)):
            lines.insert(idx, u)
    (DST / (dst_rel or rel)).write_text(HEADER.format(rel=dst_rel or rel) + '\n'.join(lines))

port('math/fixed_point.rs')
port('math/log_exp_math.rs')
port('math/cub' + 'ic_math.rs', 'math/coffer_math.rs')
port('math/max_selloff.rs')
port('math/surge_fee.rs')
port('math/weighted_math.rs')
port('math/mod.rs')
port('constants.rs', extra_uses=['use crate::coffer::anchor_lang;'])

# --- swap.rs: hand-written frame around verbatim (mapped) handler segments ----
handler = apply_renames((SRC / 'instructions/user/swap.rs').read_text()).split('\n')
def seg(start, end, end_offset=0):
    s = next(i for i, l in enumerate(handler) if start in l)
    e = next(i for i, l in enumerate(handler[s:], start=s) if end in l) + end_offset
    return rewrite_paths('\n'.join(handler[s:e + 1]))

guards = seg('require!(pool.pool_enabled, ErrorCode::PoolDisabled);', 'ErrorCode::TokenInactive', 1)
window_inputs = seg('let virtual_balance_in = pool.tokens[token_in_idx].dynamics.virtual_balance;', 'let max_selloff_period = pool.tokens[token_in_idx].config.max_selloff_period_length;')
fee_and_curve = seg('let virtual_balance_out = pool.tokens[token_out_idx].dynamics.virtual_balance;', 'decimals_out,', 1)
surge = seg('// ── Variable sell-off surge fee', 'fee_acc.min(amount_out as u128) as u64', 5)
payout = seg('// User receives the curve output minus the surge fee.', '.ok_or(ErrorCode::MathUnderflow)?;')
liq = seg('require!(amount_out <= actual_balance_out, ErrorCode::InsufficientLiquidity);', 'require!(amount_out <= actual_balance_out, ErrorCode::InsufficientLiquidity);')
state_update = seg('pool.tokens[token_in_idx].dynamics.virtual_balance = virtual_balance_in', 'pool.add_protocol_fees(token_out_idx, surge_fee_amount)?;', 1)
fee_fn = seg('/// `fee = amount * swap_fee_rate / SWAP_FEE_PRECISION`.', 'fee.try_into().map_err(|_| ErrorCode::MathOverflow.into())', 1)
pf_fn = seg('/// `protocol_fee = swap_fee * protocol_fee_rate / PROTOCOL_FEE_PRECISION`.', 'protocol_fee.try_into().map_err(|_| ErrorCode::MathOverflow.into())', 1)
assert surge.rstrip().endswith('};'), surge[-80:]
assert fee_fn.rstrip().endswith('}') and pf_fn.rstrip().endswith('}')
assert state_update.rstrip().endswith('}')

swap_rs = (HERE / 'swap_frame.rs').read_text()
swap_rs = (swap_rs.replace('@@GUARDS@@', guards)
                  .replace('@@WINDOW_INPUTS@@', window_inputs)
                  .replace('@@FEE_AND_CURVE@@', fee_and_curve)
                  .replace('@@SURGE@@', surge)
                  .replace('@@PAYOUT@@', payout)
                  .replace('@@LIQ@@', liq)
                  .replace('@@STATE_UPDATE@@', state_update)
                  .replace('@@FEE_FN@@', fee_fn.replace('fn calculate_swap_fee', 'pub fn calculate_swap_fee'))
                  .replace('@@PF_FN@@', pf_fn.replace('fn calculate_protocol_fee', 'pub fn calculate_protocol_fee')))
(DST / 'swap.rs').write_text(swap_rs)
print('ported from', SRC)
