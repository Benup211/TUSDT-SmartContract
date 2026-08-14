# Oracle — Error catalog

**Contract:** `contracts/tusdt-oracle/lib.rs` (package `tusdt-oracle`, artifact `tusdt_oracle`)

**Enum:** `pub enum Error` — all variants fieldless, `#[derive(Debug, PartialEq, Eq)]` + `#[ink::scale_derive(Encode, Decode, TypeInfo)]`. 13 variants.

**Enum doc comment (verbatim):**

> Errors returned by the oracle contract. All variants are fieldless; they group into authorization failures (NotController/NotGovernance/NotValidator), submitter eligibility (InvalidHotkey/NotRegisteredInSubnet/InsufficientStake), price and round validation (InvalidPrice/NotEnoughSubmissions/MedianUnavailable/MaxSubmissionsReached/PriceDeviationExceeded), and infrastructure failures (ChainExtensionFailed/ArithmeticError).

Fieldless `Error` enum: each variant encodes as a single `u8` SCALE index. See [index.md](index.md) for how errors surface in dedot/dApp/CLI clients.

## Variants

### `NotController`

> Caller is not the configured controller (vault).

**Returned by:** `ensure_controller` (helper)

**Client guidance:** Authorization: only the vault (controller) may call; report if a user call hits it.

### `NotGovernance`

> Caller is not the governance account.

**Returned by:** `ensure_governance` (helper)

**Client guidance:** Authorization: only governance may call; do not retry.

### `NotValidator`

> Caller is not the configured validator.

**Returned by:** `ensure_validator` (helper)

**Client guidance:** Authorization: only the configured validator may call; do not retry.

### `InvalidHotkey`

> The supplied hotkey is invalid (e.g. a zero address).

**Returned by:** `submit_price` (message)

**Client guidance:** Input error: submit with a valid non-zero hotkey.

### `NotRegisteredInSubnet`

> The caller's (coldkey, hotkey) pair is not registered in the governing subnet.

**Returned by:** `submit_price` (message)

**Client guidance:** Eligibility: register the (coldkey, hotkey) pair in the governing subnet, then retry.

### `InsufficientStake`

> The caller's subnet alpha stake is below the required minimum.

**Returned by:** `submit_price` (message)

**Client guidance:** Eligibility: ensure the caller's subnet alpha stake meets the minimum, then retry.

### `ChainExtensionFailed`

> Chain extension call failed at the node level.

**Returned by:** `submit_price` (message)

**Client guidance:** Node-level failure: retry (may be transient); report if it persists.

### `InvalidPrice`

> Submitted or overridden price was zero / invalid.

**Returned by:** `commit_round` (message), `commit_round_governance` (message), `submit_price` (message)

**Client guidance:** Input error: submit/commit a positive, non-zero price.

### `NotEnoughSubmissions`

> Round has fewer than `MIN_REPORTERS` submissions for a non-override commit.

**Returned by:** `commit_round` (message)

**Client guidance:** Quorum not reached for a non-override commit: wait for more validators to submit, or use a governance commit.

### `MedianUnavailable`

> Round contains no submissions, so a median cannot be computed.

**Returned by:** `commit_round` (message), `compute_round_median` (helper)

**Client guidance:** No submissions in the round to compute a median: report if submissions exist on-chain.

### `MaxSubmissionsReached`

> The per-round submission cap has been reached.

**Returned by:** `submit_price` (message)

**Client guidance:** The round is full: wait for the next round.

### `PriceDeviationExceeded`

> The candidate price moved outside the configured deviation band.

**Returned by:** `ensure_within_deviation` (helper)

**Client guidance:** The price moved outside the deviation band: wait for the band to widen, or use a governance commit to override.

### `ArithmeticError`

> Arithmetic overflow or underflow.

**Returned by:** `submit_price` (message), `compute_round_median` (helper), `ensure_within_deviation` (helper), `finalize_round` (helper)

**Client guidance:** Unexpected numeric error: report as a contract bug.
