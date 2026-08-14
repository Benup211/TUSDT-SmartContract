# Lending Pool — Error catalog

**Contract:** `contracts/tusdt-lending-pool/lib.rs` (package `tusdt-lending-pool`, artifact `tusdt_lending_pool`)

**Enum:** `pub enum Error` — all variants fieldless, `#[derive(Debug, PartialEq, Eq)]` + `#[ink::scale_derive(Encode, Decode, TypeInfo)]`. 43 variants.

**Enum doc comment (verbatim):**

> Error types for the lending pool. All variants are fieldless for compact SCALE encoding.

Fieldless `Error` enum: each variant encodes as a single `u8` SCALE index. See [index.md](index.md) for how errors surface in dedot/dApp/CLI clients.

**Reserved variants** (declared, not returned by any current message): `RepayAmountTooHigh`, `NoAlphaStakeFound`, `OracleCallFailed`, `InvalidCollateralNetuid`, `CloseFactorExceeded`, `PositionNotFound`.

## Variants

### `NotGovernance`

*(no doc comment in source)*

**Group:** Access

**Returned by:** `cancel_global_params_update` (message), `maintainer` (message), `update_governance` (message), `ensure_governance` (helper), `update_position_key` (helper)

**Client guidance:** Authorization: only governance may call.

### `NotGovernanceOrPlatform`

*(no doc comment in source)*

**Group:** Access

**Returned by:** `update_pool_hotkey` (message), `ensure_governance` (helper), `ensure_governance_or_platform` (helper)

**Client guidance:** Authorization: only governance or the platform may call.

### `NotMaintainer`

*(no doc comment in source)*

**Group:** Access

**Returned by:** `cancel_alpha_params_update` (message), `cancel_market_params_update` (message), `claim_surplus_tusdt` (message), `execute_global_params_update` (message), `execute_market_params_update` (message), `pause` (message), `unpause` (message), `update_oracle_address` (message), `update_platform` (message), `update_treasury` (message), `ensure_governance_or_platform` (helper), `ensure_maintainer` (helper)

**Client guidance:** Authorization: only the maintainer may call.

### `ContractPaused`

*(no doc comment in source)*

**Group:** Access

**Returned by:** `ensure_maintainer` (helper), `ensure_not_paused` (helper)

**Client guidance:** State error: the pool is paused - wait for unpause.

### `Reentrancy`

*(no doc comment in source)*

**Group:** Access

**Returned by:** `ensure_idle` (helper), `ratio_sub` (helper)

**Client guidance:** Guard: a reentrant call was blocked; report if a legitimate flow triggers it.

### `ZeroAmount`

*(no doc comment in source)*

**Group:** Amounts & liquidity

**Returned by:** `borrow_tao` (message), `borrow_tusdt` (message), `deposit_alpha` (message), `liquidate` (message), `supply_tao` (message), `supply_tusdt` (message), `withdraw_alpha` (message), `withdraw_tao` (message), `withdraw_tusdt` (message)

**Client guidance:** Input error: the amount must be greater than zero.

### `LiquidityInsufficient`

*(no doc comment in source)*

**Group:** Amounts & liquidity

**Returned by:** `borrow_tao` (message), `borrow_tusdt` (message), `withdraw_tao` (message), `withdraw_tusdt` (message)

**Client guidance:** State error: not enough liquidity in the market - lower the amount or wait for suppliers.

### `MintBelowPrecision`

*(no doc comment in source)*

**Group:** Amounts & liquidity

**Returned by:** `supply_tao` (message), `supply_tusdt` (message), `withdraw_tao` (message), `withdraw_tusdt` (message)

**Client guidance:** Input error: the amount is too small to mint lTokens at the current exchange rate - increase it.

### `InsufficientLTokenBalance`

*(no doc comment in source)*

**Group:** Amounts & liquidity

**Returned by:** `withdraw_tao` (message), `withdraw_tusdt` (message)

**Client guidance:** Input error: the user holds fewer lTokens than the withdrawal amount.

### `SupplyCapExceeded`

*(no doc comment in source)*

**Group:** Amounts & liquidity

**Returned by:** `deposit_alpha` (message), `supply_tao` (message), `supply_tusdt` (message)

**Client guidance:** State error: the market's supply cap is reached - wait or use another market.

### `BorrowCapExceeded`

*(no doc comment in source)*

**Group:** Amounts & liquidity

**Returned by:** `borrow_tao` (message), `borrow_tusdt` (message)

**Client guidance:** State error: the market's borrow cap is reached - wait for repayments.

### `BorrowHealthExceeded`

*(no doc comment in source)*

**Group:** Amounts & liquidity

**Returned by:** `borrow_tao` (message), `borrow_tusdt` (message)

**Client guidance:** Input error: the borrow would breach the collateral factor - reduce the amount or add collateral.

### `RepayAmountTooHigh`

*(no doc comment in source)*

**Group:** Amounts & liquidity

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but never returned (repay clamps to the current debt). Report if observed.

### `UnapprovedNetuid`

*(no doc comment in source)*

**Group:** Alpha collateral

**Returned by:** `deposit_alpha` (message), `liquidate` (message), `withdraw_alpha` (message), `alpha_price_rao_to_ratio` (helper), `effective_alpha` (helper), `ensure_approved_netuid` (helper), `ensure_not_paused` (helper)

**Client guidance:** Input error: use a netuid from the approved set.

### `NetuidHasPositions`

*(no doc comment in source)*

**Group:** Alpha collateral

**Returned by:** `set_approved_netuid` (message)

**Client guidance:** State error: unwind positions on the netuid before removing it (maintainer action).

### `InsufficientCollateral`

*(no doc comment in source)*

**Group:** Alpha collateral

**Returned by:** `withdraw_alpha` (message)

**Client guidance:** Input error: withdrawing more alpha collateral than held - reduce the amount.

### `InsufficientAvailableStake`

*(no doc comment in source)*

**Group:** Alpha collateral

**Returned by:** `withdraw_alpha` (message)

**Client guidance:** State error: the user's subnet stake is below what the withdrawal needs - restake and retry.

### `StakeTransferFailed`

*(no doc comment in source)*

**Group:** Alpha collateral

**Returned by:** `deposit_alpha` (message)

**Client guidance:** The alpha stake pull failed: check the user's stake, the chain minimum, and the subnet TransferToggle; retry once corrected.

### `ChainExtensionFailed`

*(no doc comment in source)*

**Group:** Alpha collateral

**Returned by:** `claim_alpha_yield` (message), `liquidate` (message), `update_pool_hotkey` (message), `withdraw_alpha` (message), `collateral_price` (helper), `get_oracle_price` (helper)

**Client guidance:** Node-level failure: retry (may be transient); report if persistent.

### `NoAlphaStakeFound`

*(no doc comment in source)*

**Group:** Alpha collateral

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but not returned by any current message. Report if observed on-chain.

### `TooManyNetuids`

*(no doc comment in source)*

**Group:** Alpha collateral

**Returned by:** `update_pool_hotkey` (message)

**Client guidance:** Input error: pass at most 32 netuids.

### `OracleCallFailed`

*(no doc comment in source)*

**Group:** Pricing & health

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but never returned (pricing currently flows through the chain extension). Report if observed.

### `OraclePriceUnavailable`

*(no doc comment in source)*

**Group:** Pricing & health

**Returned by:** `get_collateral_value_tusdt` (message), `get_oracle_price` (helper), `market_cash` (helper)

**Client guidance:** State error: no price available - wait for a fresh oracle submission and retry.

### `OraclePriceStale`

*(no doc comment in source)*

**Group:** Pricing & health

**Returned by:** `get_collateral_value_tusdt` (message), `get_oracle_price` (helper), `market_cash` (helper)

**Client guidance:** State error: the price is older than the max age - wait for a fresh submission and retry.

### `HealthFactorBelowThreshold`

*(no doc comment in source)*

**Group:** Pricing & health

**Returned by:** `withdraw_alpha` (message)

**Client guidance:** Input error: the withdrawal would push the health factor below the threshold - reduce the amount.

### `NotLiquidatable`

*(no doc comment in source)*

**Group:** Pricing & health

**Returned by:** `liquidate` (message)

**Client guidance:** State error: the position's health factor is above the threshold - not liquidatable.

### `InvalidCollateralNetuid`

*(no doc comment in source)*

**Group:** Pricing & health

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but not returned by any current message. Report if observed on-chain.

### `InvalidDebtMarket`

*(no doc comment in source)*

**Group:** Pricing & health

**Returned by:** `liquidate` (message)

**Client guidance:** Input error: liquidations must reference a valid debt market id.

### `CloseFactorExceeded`

*(no doc comment in source)*

**Group:** Pricing & health

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but not returned by any current message. Report if observed on-chain.

### `CollateralAwardExceedsPosition`

*(no doc comment in source)*

**Group:** Pricing & health

**Returned by:** `liquidate` (message)

**Client guidance:** Unexpected liquidation math: report as a contract bug (should not occur with valid inputs).

### `InvalidRatio`

*(no doc comment in source)*

**Group:** Params / timelock

**Returned by:** `cancel_alpha_params_update` (message), `cancel_market_params_update` (message), `set_global_params` (message), `set_market_params` (message), `alpha_params_from_config` (helper), `global_params_from_config` (helper), `interest_params_from_config` (helper), `validate_alpha_params` (helper), `validate_global_params` (helper), `validate_interest_params` (helper)

**Client guidance:** Input error: a ratio parameter failed validation - fix it and resubmit.

### `InvalidParam`

*(no doc comment in source)*

**Group:** Params / timelock

**Returned by:** `cancel_alpha_params_update` (message), `cancel_market_params_update` (message), `is_liquidatable` (message), `set_global_params` (message), `set_market_params` (message), `alpha_params_from_config` (helper), `global_params_from_config` (helper), `interest_params_from_config` (helper), `validate_alpha_params` (helper), `validate_interest_params` (helper)

**Client guidance:** Input error: a parameter failed validation - fix it and resubmit.

### `NoPendingMarketParamsUpdate`

*(no doc comment in source)*

**Group:** Params / timelock

**Returned by:** `cancel_market_params_update` (message), `execute_market_params_update` (message), `set_market_params` (message)

**Client guidance:** Idempotency: no pending update for this market - schedule one first.

### `NoPendingAlphaParamsUpdate`

*(no doc comment in source)*

**Group:** Params / timelock

**Returned by:** `cancel_alpha_params_update` (message), `execute_alpha_params_update` (message)

**Client guidance:** Idempotency: no pending alpha-params update - schedule one first.

### `NoPendingGlobalParamsUpdate`

*(no doc comment in source)*

**Group:** Params / timelock

**Returned by:** `cancel_global_params_update` (message), `execute_global_params_update` (message), `set_global_params` (message)

**Client guidance:** Idempotency: no pending global-params update - schedule one first.

### `ParamsUpdateTimelockActive`

*(no doc comment in source)*

**Group:** Params / timelock

**Returned by:** `execute_alpha_params_update` (message), `execute_global_params_update` (message), `execute_market_params_update` (message), `set_global_params` (message), `set_market_params` (message)

**Client guidance:** Timing: the update timelock has not expired - wait and retry.

### `TokenContractCallFailed`

*(no doc comment in source)*

**Group:** Cross-contract

**Returned by:** `borrow_tusdt` (message), `claim_reserve` (message), `claim_surplus_tusdt` (message), `unpause` (message), `withdraw_tusdt` (message)

**Client guidance:** Cross-contract failure to the ERC20 token: retry; report if persistent.

### `TokenTransferFromFailed`

*(no doc comment in source)*

**Group:** Cross-contract

**Returned by:** `liquidate` (message), `repay_tusdt` (message), `supply_tusdt` (message)

**Client guidance:** The ERC20 `transferFrom` failed: check the user's allowance/balance and retry.

### `LTokenCallFailed`

*(no doc comment in source)*

**Group:** Cross-contract

**Returned by:** `supply_tao` (message), `supply_tusdt` (message), `withdraw_tao` (message), `withdraw_tusdt` (message)

**Client guidance:** Cross-contract failure to the lToken: retry; report if persistent.

### `TransferFailed`

*(no doc comment in source)*

**Group:** Cross-contract

**Returned by:** `borrow_tao` (message), `claim_alpha_yield` (message), `claim_reserve` (message), `claim_surplus_tusdt` (message), `liquidate` (message), `repay_tao` (message), `supply_tao` (message), `transfer_native_to_treasury` (message), `withdraw_tao` (message)

**Client guidance:** A native TAO transfer failed: check the balance and retry.

### `MarketNotFound`

*(no doc comment in source)*

**Group:** General

**Returned by:** `borrow_tao` (message), `borrow_tusdt` (message), `cancel_alpha_params_update` (message), `claim_reserve` (message), `get_collateral_value_tusdt` (message), `get_debt_value_tusdt` (message), `liquidate` (message), `repay_tao` (message), `repay_tusdt` (message), `set_market_params` (message), `supply_tao` (message), `supply_tusdt` (message), `update_ltoken_address` (message), `update_oracle_address` (message), `withdraw_tao` (message), `withdraw_tusdt` (message), `accrue_interest` (helper), `div_ratio` (helper), `effective_alpha` (helper), `ensure_approved_netuid` (helper), `market_cash` (helper), `max_liquidation_threshold_for_user` (helper), `min_collateral_factor_for_user` (helper)

**Client guidance:** Input error: unknown market id - query the supported markets and retry.

### `PositionNotFound`

*(no doc comment in source)*

**Group:** General

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but not returned by any current message. Report if observed on-chain.

### `ArithmeticError`

*(no doc comment in source)*

**Group:** General

**Returned by:** `borrow_tao` (message), `borrow_tusdt` (message), `cancel_alpha_params_update` (message), `cancel_market_params_update` (message), `claim_alpha_yield` (message), `claim_reserve` (message), `deposit_alpha` (message), `get_available_borrow_tusdt` (message), `get_collateral_value_tusdt` (message), `get_debt_value_tusdt` (message), `get_health_factor` (message), `is_liquidatable` (message), `liquidate` (message), `repay_tao` (message), `repay_tusdt` (message), `set_alpha_params` (message), `set_approved_netuid` (message), `set_global_params` (message), `set_market_params` (message), `supply_tao` (message), `supply_tusdt` (message), `transfer_native_to_treasury` (message), `withdraw_alpha` (message), `withdraw_tao` (message), `withdraw_tusdt` (message), `accrue_interest` (helper), `alpha_price_rao_to_ratio` (helper), `collateral_price` (helper), `div_ratio` (helper), `effective_alpha` (helper), `ensure_approved_netuid` (helper), `get_oracle_price` (helper), `interest_params_from_config` (helper), `market_cash` (helper), `max_liquidation_threshold_for_user` (helper), `min_collateral_factor_for_user` (helper), `validate_interest_params` (helper)

**Client guidance:** Unexpected numeric error: report as a contract bug.
