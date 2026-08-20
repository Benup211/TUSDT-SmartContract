# Governance — Error catalog

**Contract:** `contracts/tusdt-governance/lib.rs` (package `tusdt-governance`, artifact `tusdt_governance`)

**Enum:** `pub enum Error` — all variants fieldless, `#[derive(Debug, PartialEq, Eq)]` + `#[ink::scale_derive(Encode, Decode, TypeInfo)]`. 27 variants.

**Enum doc comment (verbatim):**

> Errors returned by the governance contract.

Fieldless `Error` enum: each variant encodes as a single `u8` SCALE index. See [index.md](index.md) for how errors surface in dedot/dApp/CLI clients.

**Reserved variants** (declared, not returned by any current message): `InsufficientStake`, `StakeQueryFailed`.

## Variants

### `NotMaintainer`

*(no doc comment in source)*

**Returned by:** `ensure_maintainer` (helper)

**Client guidance:** Authorization: only the maintainer may call.

### `NotElection`

*(no doc comment in source)*

**Returned by:** `ensure_election` (helper)

**Client guidance:** Authorization: only the election contract may call; report if a user call hits it.

### `NotCouncil`

*(no doc comment in source)*

**Returned by:** `ensure_council` (helper)

**Client guidance:** Authorization: only current council members may call; check council membership.

### `InvalidCouncil`

*(no doc comment in source)*

**Returned by:** `set_council` (message), `update_treasury_address` (message)

**Client guidance:** Input error: the council set is invalid (wrong size / duplicates); the maintainer must submit a valid council.

### `ProposalNotFound`

*(no doc comment in source)*

**Returned by:** `execute` (message), `finalize` (message), `vote` (message)

**Client guidance:** Check the proposal id against on-chain proposals; retry with a valid id.

### `ProposalNotActive`

*(no doc comment in source)*

**Returned by:** `finalize` (message), `vote` (message)

**Client guidance:** State error: the proposal is not in the active phase; check status before voting/finalizing.

### `VotingClosed`

*(no doc comment in source)*

**Returned by:** `vote` (message)

**Client guidance:** State error: voting ended - no further votes are accepted.

### `VotingStillOpen`

*(no doc comment in source)*

**Returned by:** `finalize` (message)

**Client guidance:** State error: wait for voting to end before finalizing.

### `AlreadyVoted`

*(no doc comment in source)*

**Returned by:** `vote` (message)

**Client guidance:** Idempotency: this (coldkey, hotkey) pair already voted; do not retry.

### `AlreadyExecuted`

*(no doc comment in source)*

**Returned by:** `execute` (message)

**Client guidance:** Idempotency: treat as success - the proposal was already executed.

### `NoStake`

*(no doc comment in source)*

**Returned by:** `vote` (message)

**Client guidance:** Eligibility: the caller has zero voting power in the snapshot.

### `InsufficientStake`

*(no doc comment in source)*

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but not returned by any current message. Report if observed on-chain.

### `OutsideSubmissionWindow`

*(no doc comment in source)*

**Returned by:** `submit_proposal` (message)

**Client guidance:** Timing: submit proposals only within the submission window.

### `NoSnapshot`

*(no doc comment in source)*

**Returned by:** `submit_proposal` (message), `vote` (message)

**Client guidance:** State error: wait for the council to commit an electorate snapshot.

### `InvalidProof`

*(no doc comment in source)*

**Returned by:** `vote` (message)

**Client guidance:** Input error: the Merkle proof does not match the snapshot root - recompute from the committed root.

### `StakeQueryFailed`

*(no doc comment in source)*

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but not returned by any current message. Report if observed on-chain.

### `InvalidCid`

*(no doc comment in source)*

**Returned by:** `submit_proposal` (message)

**Client guidance:** Input error: provide a valid IPFS CID for the proposal description.

### `InvalidAmount`

*(no doc comment in source)*

**Returned by:** `submit_proposal` (message)

**Client guidance:** Input error: the amount is invalid (e.g. zero); fix and resubmit.

### `InvalidParams`

*(no doc comment in source)*

**Returned by:** `update_params` (message)

**Client guidance:** Input error: params failed validation bounds; fix and resubmit.

### `NotPassed`

*(no doc comment in source)*

**Returned by:** `execute` (message)

**Client guidance:** State error: the proposal did not meet quorum; it cannot execute.

### `TreasuryCallFailed`

*(no doc comment in source)*

**Returned by:** `execute` (message)

**Client guidance:** Cross-contract failure to the treasury: check treasury state; retry; report if persistent.

### `VaultCallFailed`

*(no doc comment in source)*

**Returned by:** `forward_vault_cancel_contract_params_update` (helper, external_calls.rs), `forward_vault_cancel_global_params_update` (helper, external_calls.rs), `forward_vault_claim_excess_alpha` (helper, external_calls.rs), `forward_vault_pause` (helper, external_calls.rs), `forward_vault_set_approved_netuid` (helper, external_calls.rs), `forward_vault_set_contract_params` (helper, external_calls.rs), `forward_vault_set_global_params` (helper, external_calls.rs), `forward_vault_set_hotkey` (helper, external_calls.rs), `forward_vault_transfer_native_to_treasury` (helper, external_calls.rs), `forward_vault_unpause` (helper, external_calls.rs), `forward_vault_update_auction_address` (helper, external_calls.rs), `forward_vault_update_oracle_address` (helper, external_calls.rs), `forward_vault_update_platform` (helper, external_calls.rs), `forward_vault_update_treasury` (helper, external_calls.rs)

**Client guidance:** Cross-contract failure to a vault forwarder: check the vault's params/state, then retry.

### `AuctionCallFailed`

*(no doc comment in source)*

**Returned by:** `forward_auction_set_admin` (helper, external_calls.rs)

**Client guidance:** Cross-contract failure to the auction contract: retry; report if persistent.

### `PoolCallFailed`

*(no doc comment in source)*

**Returned by:** `forward_pool_cancel_alpha_params_update` (helper, external_calls.rs), `forward_pool_cancel_global_params_update` (helper, external_calls.rs), `forward_pool_cancel_market_params_update` (helper, external_calls.rs), `forward_pool_claim_surplus_tusdt` (helper, external_calls.rs), `forward_pool_set_alpha_params` (helper, external_calls.rs), `forward_pool_set_approved_netuid` (helper, external_calls.rs), `forward_pool_set_global_params` (helper, external_calls.rs), `forward_pool_set_market_params` (helper, external_calls.rs), `forward_pool_transfer_native_to_treasury` (helper, external_calls.rs), `forward_pool_unpause` (helper, external_calls.rs), `forward_pool_update_ltoken_address` (helper, external_calls.rs), `forward_pool_update_maintainer` (helper, external_calls.rs), `forward_pool_update_oracle_address` (helper, external_calls.rs), `forward_pool_update_platform` (helper, external_calls.rs), `forward_pool_update_pool_hotkey` (helper, external_calls.rs), `forward_pool_update_treasury` (helper, external_calls.rs)

**Client guidance:** Cross-contract failure to the lending pool: check the pool's params/state, then retry.

### `OracleCallFailed`

*(no doc comment in source)*

**Returned by:** `forward_oracle_commit_round` (helper, external_calls.rs), `forward_oracle_set_max_price_deviation` (helper, external_calls.rs), `forward_oracle_set_min_submitter_stake` (helper, external_calls.rs), `forward_oracle_set_netuid` (helper, external_calls.rs), `forward_oracle_set_validator` (helper, external_calls.rs)

**Client guidance:** Cross-contract failure to the oracle: retry; report if persistent.

### `VaultTokenCallFailed`

> Cross-contract call to the vault's token-controller transfer failed.

**Returned by:** `forward_vault_set_token_controller` (helper, external_calls.rs)

**Client guidance:** Cross-contract failure in the vault's token-controller transfer: check approvals and retry.

### `ArithmeticError`

*(no doc comment in source)*

**Returned by:** `finalize` (message), `submit_proposal` (message), `submit_snapshot` (message), `vote` (message)

**Client guidance:** Unexpected numeric error: report as a contract bug.
