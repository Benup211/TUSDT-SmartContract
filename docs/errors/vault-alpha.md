# Alpha Vault — Error catalog

**Contract:** `contracts/tusdt-vault-alpha/lib.rs` (package `tusdt-vault-alpha`, artifact `tusdt_vault_alpha`)

**Enum:** `pub enum Error` — all variants fieldless, `#[derive(Debug, PartialEq, Eq)]` + `#[ink::scale_derive(Encode, Decode, TypeInfo)]`. 38 variants.

**Enum doc comment (verbatim):**

> Errors returned by the alpha vault contract.

Fieldless `Error` enum: each variant encodes as a single `u8` SCALE index. See [index.md](index.md) for how errors surface in dedot/dApp/CLI clients.

**Reserved variants** (declared, not returned by any current message): `NotVaultOwner`, `InvalidTransactionFee`, `TokenBorrowedNotZero`, `LiquidationRatioExceeded`.

## Variants

### `VaultNotFound`

> No vault exists for the given (owner, vault_id) pair.

**Returned by:** `get_all_vaults` (message), `get_max_borrow` (message), `get_vault_collateral_value` (message), `get_vaults` (message), `load_caller_vault` (helper, vault_access.rs), `load_vault` (helper, vault_access.rs)

**Client guidance:** Check the (owner, vault_id) pair against `get_vaults` / `get_all_vaults`; retry with a valid id.

### `InsufficientCollateral`

> Collateral amount is zero or insufficient for the operation.

**Returned by:** `create_alpha_vault` (message), `release_alpha_collateral` (message), `ensure_collateral_bounds` (helper, risk.rs)

**Client guidance:** Input error: pass a non-zero collateral amount within the collateral bounds.

### `NotVaultOwner`

> The caller does not own the specified vault.

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but not returned by any current message. If observed on a live deployment, the deployed code differs from this source - report.

### `TransferFailed`

> A token or native TAO transfer failed.

**Returned by:** `borrow_token` (message), `claim_excess_alpha` (message), `claim_surplus_tusdt` (message), `create_alpha_vault` (message), `repay_token` (message), `settle_liquidation_auction` (message), `transfer_native_to_treasury` (message), `trigger_liquidation_auction` (message), `transfer_transaction_fee_to_treasury` (helper)

**Client guidance:** A token or native TAO transfer failed: check balances, retry; report if balances look correct.

### `InsufficientTokenBalance`

> The caller does not hold enough TUSDT balance.

**Returned by:** `ensure_token_balance_at_least` (helper)

**Client guidance:** Input error: the caller lacks TUSDT balance; top up or reduce the amount.

### `InvalidTransactionFee`

> Transaction fee exceeds 100%.

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but never returned (fee validation is enforced via `InvalidRatio`). Report if observed.

### `TokenBorrowedNotZero`

> Vault still has outstanding borrowed tokens that must be repaid first.

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but never returned. Report if observed.

### `InvalidRatio`

> A ratio parameter failed validation bounds.

**Returned by:** `validate_contract_params` (helper, params.rs), `validate_global_params` (helper, params.rs)

**Client guidance:** Input error: fix the ratio parameter to respect validation bounds and retry.

### `InvalidAuctionDuration`

> Auction duration is outside the allowed [60 s, 7 d] range.

**Returned by:** `validate_global_params` (helper, params.rs)

**Client guidance:** Input error: pass an auction duration within [60 s, 7 d].

### `CollateralRatioExceeded`

> The resulting debt would exceed the maximum allowed by the collateral ratio.

**Returned by:** `borrow_token` (message), `release_alpha_collateral` (message)

**Client guidance:** Input error: borrow/release would exceed the collateral ratio - reduce the amount or add collateral.

### `LiquidationRatioExceeded`

> The resulting debt would exceed the liquidation threshold.

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but never returned. Report if observed.

### `RepayAmountTooHigh`

> Repayment amount exceeds the vault's current debt balance.

**Returned by:** `repay_token` (message)

**Client guidance:** Input error: repay no more than the vault's current debt.

### `VaultInLiquidation`

> The vault is currently in liquidation.

**Returned by:** `ensure_not_in_liquidation` (helper, vault_access.rs)

**Client guidance:** State error: wait for the liquidation auction to settle before operating on this vault.

### `NotLiquidatable`

> The vault does not meet the liquidation criteria (debt not above threshold).

**Returned by:** `trigger_liquidation_auction` (message)

**Client guidance:** State error: the vault's debt is below the liquidation threshold; no action.

### `LiquidationAuctionExists`

> A liquidation auction already exists for this vault.

**Returned by:** `trigger_liquidation_auction` (message)

**Client guidance:** State error: an auction is already open for this vault; do not trigger another.

### `AuctionContractCallFailed`

> Cross-contract call to the auction contract failed.

**Returned by:** `settle_liquidation_auction` (message), `trigger_liquidation_auction` (message), `sync_child_governance` (helper)

**Client guidance:** Cross-contract failure to the auction contract: retry; report if persistent.

### `AuctionNotFound`

> No auction found for the given ID.

**Returned by:** `settle_liquidation_auction` (message)

**Client guidance:** State error: no auction for the given id - check the id and retry.

### `AuctionNotFinalized`

> The auction has not yet been finalized.

**Returned by:** `settle_liquidation_auction` (message)

**Client guidance:** State error: wait for the auction to be finalized before settling.

### `ArithmeticError`

> Arithmetic overflow, underflow, or conversion failure.

**Returned by:** `add_alpha_collateral` (message), `borrow_token` (message), `claim_excess_alpha` (message), `create_alpha_vault` (message), `release_alpha_collateral` (message), `repay_token` (message), `set_contract_params` (message), `set_global_params` (message), `settle_liquidation_auction` (message), `trigger_liquidation_auction` (message), `alpha_price_rao_to_ratio` (helper), `calculate_transaction_fee` (helper), `collateral_value` (helper, risk.rs), `current_collateral_price` (helper), `ensure_collateral_bounds` (helper, risk.rs), `get_contract_stake` (helper), `liquidation_limit` (helper, risk.rs), `liquidation_min_bid` (helper, risk.rs), `max_borrow_allowed` (helper, risk.rs), `sync_owner_total_debt` (helper), `validate_price_data` (helper)

**Client guidance:** Unexpected numeric error: report as a contract bug.

### `NotGovernance`

> The caller is not the governance address.

**Returned by:** `ensure_governance` (helper)

**Client guidance:** Authorization: only governance may call.

### `NotGovernanceOrPlatform`

> The caller is neither the governance nor the platform address.

**Returned by:** `ensure_governance_or_platform` (helper)

**Client guidance:** Authorization: only governance or the platform may call.

### `ContractPaused`

> The contract is paused.

**Returned by:** `ensure_not_paused` (helper)

**Client guidance:** State error: the contract is paused - wait for unpause by governance.

### `OracleCallFailed`

> Cross-contract call to the oracle failed.

**Returned by:** `sync_child_governance` (helper)

**Client guidance:** Cross-contract failure to the oracle: retry; report if persistent.

### `OraclePriceUnavailable`

> No price data available from the oracle.

**Returned by:** `validate_price_data` (helper)

**Client guidance:** No price data from the oracle: wait for a fresh oracle commit and retry.

### `OraclePriceStale`

> The oracle price data has exceeded the maximum allowable age.

**Returned by:** `validate_price_data` (helper)

**Client guidance:** The oracle price is older than the max age: wait for a fresh commit and retry.

### `InvalidOracleMaxAge`

> The max oracle age parameter is invalid (e.g., zero).

**Returned by:** `validate_global_params` (helper, params.rs)

**Client guidance:** Input error: the max oracle age must be non-zero; fix the param.

### `NoPendingContractParamsUpdate`

> No pending parameter update for the given netuid.

**Returned by:** `cancel_contract_params_update` (message), `execute_contract_params_update` (message)

**Client guidance:** Idempotency: no pending update for the netuid - schedule one first (or treat as already cancelled/executed).

### `NoPendingGlobalParamsUpdate`

> No pending global-parameter update.

**Returned by:** `cancel_global_params_update` (message), `execute_global_params_update` (message)

**Client guidance:** Idempotency: no pending global update - schedule one first (or treat as already cancelled/executed).

### `ContractParamsUpdateTimelockActive`

> The timelock for the pending parameter update has not yet expired.

**Returned by:** `execute_contract_params_update` (message), `execute_global_params_update` (message)

**Client guidance:** Timing: the timelock has not expired; wait and retry.

### `ChainExtensionFailed`

> Chain extension call failed at the node level.

**Returned by:** `claim_excess_alpha` (message), `release_alpha_collateral` (message), `set_vault_hotkey` (message), `current_collateral_price` (helper), `get_contract_stake` (helper)

**Client guidance:** Node-level failure: retry (may be transient); report if it persists.

### `NoAlphaStakeFound`

> No alpha stake found for the vault's configured (hotkey, netuid).

**Returned by:** `claim_excess_alpha` (message), `set_vault_hotkey` (message), `get_contract_stake` (helper)

**Client guidance:** State error: the vault's (hotkey, netuid) has no alpha stake - check the vault's stake setup.

### `StakeTransferFailed`

> The caller-forwarded stake pull failed: the caller may lack sufficient stake under the vault's hotkey on that subnet, the amount may be below the chain's minimum, the subnet's TransferToggle may be off, or the runtime may not support the caller-forwarded chain-extension call.

**Returned by:** `add_alpha_collateral` (message), `create_alpha_vault` (message)

**Client guidance:** The stake pull failed: check the caller's stake under the vault hotkey, the chain minimum, and the subnet TransferToggle; retry once corrected.

### `UnapprovedNetuid`

> The specified netuid is not in the approved set.

**Returned by:** `ensure_approved_netuid` (helper)

**Client guidance:** Input error: use a netuid from the approved set (governance can approve new netuids).

### `TokenContractCallFailed`

> Cross-contract call to the ERC20 token contract failed.

**Returned by:** `set_token_controller` (message)

**Client guidance:** Cross-contract failure to the ERC20 contract: retry; report if persistent.

### `VaultCreationFeeNotMet`

> The caller did not transfer enough native TAO for the vault creation fee.

**Returned by:** `create_alpha_vault` (message)

**Client guidance:** Input error: transfer the required native TAO creation fee with the call.

### `ActiveLiquidationsExist`

> Operation blocked because one or more vaults are in active liquidation auctions.

**Returned by:** `claim_excess_alpha` (message), `ensure_no_active_liquidations` (helper)

**Client guidance:** State error: blocked while liquidation auctions are active; wait for settlement.

### `TooManyNetuids`

> Too many netuids passed to `set_vault_hotkey` (max 32).

**Returned by:** `set_vault_hotkey` (message)

**Client guidance:** Input error: pass at most 32 netuids to `set_vault_hotkey`.

### `NetuidHasPositions`

> Cannot remove a netuid that still has active collateral positions.

**Returned by:** `set_approved_netuid` (message)

**Client guidance:** State error: unwind positions on the netuid before removing it from the approved set.
