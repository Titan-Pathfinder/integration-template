# Public-chain replay fixtures

These gzip archives contain captured public Solana accounts, including deployed
programdata, not wallets or private signing material. They are used only in a
local bank with synthetic user balances. The captured Ocelot and Phoenix state
is left unchanged in parity tests; negative tests fork it explicitly.

See [the verification record](../../HANDOFF.md)
for capture slots, executable/fixture hashes, outputs, limitations, and the
quote-curve counterexample. `examples/probe.rs` writes the same archive format.

After gzip decoding, bincode deserializes a `BTreeMap<String, Vec<u8>>`. Each
address maps to bincode-encoded `Option<solana_account::Account>` using the host
crate's pinned Solana 2 account layout. The test reader loads this in memory;
it does not extract executable files or submit transactions.
