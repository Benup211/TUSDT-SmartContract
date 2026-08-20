# Contract error catalog

Reference for every `Error` variant returned by the TUSDT ink! contracts in
this workspace. One page per contract:

| Contract | Package / dir | Deploy artifact | Variants | Reserved¹ | Page |
|---|---|---|---|---|---|
| tUSDT token | `tusdt-erc20` | `tusdt_erc20` | 5 | 0 | [erc20.md](erc20.md) |
| Auction | `tusdt-auction` | `tusdt_auction` | 19 | 0 | [auction.md](auction.md) |
| Oracle | `tusdt-oracle` | `tusdt_oracle` | 13 | 0 | [oracle.md](oracle.md) |
| Alpha Vault | `tusdt-vault-alpha` | `tusdt_vault_alpha` | 38 | 4 | [vault-alpha.md](vault-alpha.md) |
| Treasury | `tusdt-treasury` | `tusdt_treasury` | 4 | 0 | [treasury.md](treasury.md) |
| Governance | `tusdt-governance` | `tusdt_governance` | 27 | 2 | [governance.md](governance.md) |
| Election | `tusdt-election` | `tusdt_election` | 22 | 1 | [election.md](election.md) |
| Lending Pool | `tusdt-lending-pool` | `tusdt_lending_pool` | 43 | 6 | [lending-pool.md](lending-pool.md) |

¹ *Reserved* = declared in the enum but not returned by any current message
(13 total). If one of these is observed against a live deployment, the
deployed code is newer/different than this source — verify behavior against
the chain, not the local ABI.

## How errors surface

All eight contracts use a **fieldless** `Error` enum:

```rust
#[derive(Debug, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub enum Error { /* ... */ }
```

Every variant encodes as a single `u8` SCALE index (0, 1, 2, … in declaration
order), which is why cross-contract callers can decode another contract's
`Result<(), Error>` layout-compatibly as `Result<(), u8>` — see the note in
[election.md](election.md#decoding-note-election---governance-calls).

In clients (dedot, tusdt-app, tusdt-cli):

- Failed messages return `Result<T, Error>`. The decode shape is **not
  uniform**: unwrap both `{ isOk, value }` and Rust-style `{ Ok: … }` /
  `{ Err: … }` shapes before deciding whether the call succeeded.
- A successful query returning `Result<Option<X>, …>` from the *deployed*
  contract can disagree with the local ABI when the deployed code differs from
  the checked-out source. When UI numbers look wrong, get ground truth from
  the live chain (CLI read-only queries hit the same chain).
- Cross-contract callers swallow callee errors into their own variants
  (`GovernanceCallFailed`, `VaultCallFailed`, `AuctionContractCallFailed`,
  `TokenContractCallFailed`, `LTokenCallFailed`, `OracleCallFailed`, …).
  When a forwarder fails, inspect the callee contract's state, not just the
  caller.

## Guidance legend

Each variant's page gives a short *client guidance* line in one of these
shapes:

- **Retry** — transient or node-level failure; retrying is safe.
- **Check inputs / fix params** — the call data was invalid; correct it and
  resubmit.
- **Wait / state error** — the contract is in a phase that forbids the action
  (auction open, timelock active, voting open, …); poll state and retry later.
- **Idempotency** — the action already happened; treat as success.
- **Authorization** — the caller is not the expected role; switch accounts or
  check the configured roles; do not retry.
- **Report** — unexpected invariant/numeric failure, or a variant that the
  current source never returns; file a bug with the call context.

## Related docs

- **What each contract does** (formulas, user flows, parameters): see the user guide at
  [`../contracts/`](../contracts/index.md).
