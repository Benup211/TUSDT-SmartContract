# Treasury — Error catalog

**Contract:** `contracts/tusdt-treasury/lib.rs` (package `tusdt-treasury`, artifact `tusdt_treasury`)

**Enum:** `pub enum Error` — all variants fieldless, `#[derive(Debug, PartialEq, Eq)]` + `#[ink::scale_derive(Encode, Decode, TypeInfo)]`. 4 variants.

**Enum doc comment (verbatim):**

> Errors returned by the treasury contract.

Fieldless `Error` enum: each variant encodes as a single `u8` SCALE index. See [index.md](index.md) for how errors surface in dedot/dApp/CLI clients.

## Variants

### `NotGovernance`

> The caller is not the governance account.

**Returned by:** `ensure_governance` (helper)

**Client guidance:** Authorization: only governance may call.

### `InsufficientFundBalance`

> The fund's balance is insufficient for the requested release amount.

**Returned by:** `release` (message)

**Client guidance:** Input error: the fund's balance is below the release amount - check the amount and retry later.

### `TransferFailed`

> The ERC20 or native transfer call failed.

**Returned by:** `release` (message)

**Client guidance:** The ERC20/native transfer failed: check balances; report if they look correct.

### `ArithmeticError`

> An arithmetic overflow or underflow occurred.

**Returned by:** `release` (message), `allocate_to_funds` (helper)

**Client guidance:** Unexpected numeric error: report as a contract bug.
