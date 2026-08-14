# tUSDT token — Error catalog

**Contract:** `contracts/tusdt-erc20/lib.rs` (package `tusdt-erc20`, artifact `tusdt_erc20`)

**Enum:** `pub enum Error` — all variants fieldless, `#[derive(Debug, PartialEq, Eq)]` + `#[ink::scale_derive(Encode, Decode, TypeInfo)]`. 5 variants.

**Enum doc comment (verbatim):**

> Errors returned by the tUSDT token contract.

Fieldless `Error` enum: each variant encodes as a single `u8` SCALE index. See [index.md](index.md) for how errors surface in dedot/dApp/CLI clients.

## Variants

### `InsufficientBalance`

> Sender's balance is below the requested amount.

**Returned by:** `burn` (message), `transfer_from_to` (helper)

**Client guidance:** Input error: check the sender's balance before calling; top up or lower the amount and retry.

### `InsufficientAllowance`

> Caller's allowance from the owner is below the requested amount.

**Returned by:** `decrease_allowance` (message), `transfer_from` (message)

**Client guidance:** Input/approval error: have the owner re-approve a higher allowance, then retry.

### `NotMinter`

> Caller is not an authorized minter account.

**Returned by:** `ensure_minter` (helper)

**Client guidance:** Authorization error: only the minter may mint. Regular callers will never succeed - do not retry.

### `NotController`

> Caller is not the configured controller account.

**Returned by:** `ensure_controller` (helper)

**Client guidance:** Authorization error: only the controller contract may call. dApp users should never hit this - report if they do.

### `ArithmeticError`

> An arithmetic overflow or underflow occurred.

**Returned by:** `burn` (message), `mint` (message), `transfer_from_to` (helper)

**Client guidance:** Unexpected numeric overflow/underflow: report as a contract bug (normal inputs should not trigger it).
