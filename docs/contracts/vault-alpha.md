# tusdt-vault-alpha — Alpha Vault (CDP Engine)

The Alpha Vault is the TUSDT protocol's collateralized debt position (CDP) engine. Deposit **alpha stake** from an approved Bittensor subnet as collateral, and borrow **TUSDT** against it. Think of it as a MakerDAO Vault on Bittensor: alpha is your collateral, TUSDT is the dai you draw, and every open position has a collateral ratio that must stay healthy or the position gets liquidated at auction.

One vault contract instance services a **single staking hotkey** across multiple governance-approved subnets (netuids). Each account may open multiple vaults, one per deposit. There is **no interest**: your debt is exactly what you borrowed, and you repay 1:1.

## How it works

**Collateral.** Collateral is alpha stake, held with the vault contract as *coldkey* under the vault's single *hotkey* (`vault_hotkey`). Deposits are **atomic pulls**: `create_alpha_vault` / `add_alpha_collateral` pull the caller's own stake into the contract's coldkey via the caller-forwarded `caller_transfer_stake` chain extension (func 25) in the same message — no two-step deposit intent, no front-running window. Requirements: the caller's stake must sit under `vault_hotkey` on that subnet, the subnet's `TransferToggle` must be on, and the amount must exceed the chain minimum (≈0.002 TAO equivalent); otherwise the call reverts with `StakeTransferFailed`.

**Pricing (two sources).** Each subnet's collateral price combines the oracle's TUSDT/TAO price with the chain's on-chain alpha/TAO price:

```text
TUSDT_per_alpha = oracle_TUSDT_per_TAO × (chain_ext_alpha_price_rao / 1_000_000_000)
```

- `oracle_TUSDT_per_TAO` — latest committed price from the tusdt-oracle contract (`PriceData.price`, 1e18 fixed-point). Rejected unless present, non-zero, and no older than `max_oracle_age_ms` (`OraclePriceUnavailable` / `OraclePriceStale`).
- `chain_ext_alpha_price_rao` — `get_alpha_price(netuid)` chain extension (func 15), TAO per alpha scaled by 1e9.

**Collateral value.** TUSDT value of a vault's alpha balance:

```text
collateral_value = TUSDT_per_alpha × collateral_balance      # rao, floors down
```

**Borrow limit.** Max debt allowed against the collateral, per-netuid ratio:

```text
max_borrow = collateral_value / collateral_ratio             # CR default 150% → /1.5, floors down
```

A borrow fails with `CollateralRatioExceeded` if the resulting debt would exceed `max_borrow`. Releasing collateral applies the same check against the projected post-release collateral.

**Liquidation trigger.** A vault becomes liquidatable the moment its debt strictly exceeds the liquidation limit:

```text
liquidatable ⇔ borrowed_token_balance > collateral_value / liquidation_ratio   # LR default 120%
```

The check is `>` (strict): sitting exactly at the limit is not liquidatable. Liquidation is **permissionless** — anyone may call `trigger_liquidation_auction(owner, vault_id)`, which unstakes *all* of the vault's alpha to native TAO (`remove_stake`, func 2) and opens an ascending-bid auction (see `tusdt-auction`). The auction's minimum bid includes the liquidation fee:

```text
min_bid = debt × (1 + liquidation_fee)                       # fee default 11%
```

**Fees.**

- **Borrow & repay are free.** No fee of any kind; no interest.
- **Transaction fee** (`transaction_fee`, default 0.3% = 30 bps): charged on the *collateral* (TAO side) only, at auction settlement — `transaction_fee × collateral_sold` goes to the treasury, the winner receives the remainder.
- **Liquidation fee** (default 11%): folded into the auction `min_bid`; the winning bid is transferred to the vault, which **burns exactly the debt** (`debt_cleared = debt at trigger`). Any surplus TUSDT (winning bid − debt) stays in the vault as surplus, claimable by governance/platform via `claim_surplus_tusdt`.
- **Vault creation fee** (`vault_creation_fee`, default 5,000,000 rao = 0.005 TAO native): paid with `create_alpha_vault`; any excess value sent is refunded to the caller.

**Liquidation safety gate.** `active_liquidation_count` tracks vaults in open liquidation auctions. While non-zero, three governance operations are blocked (`ActiveLiquidationsExist`): `claim_excess_alpha`, `set_vault_hotkey`, and `transfer_native_to_treasury` — native TAO from liquidation must stay in the contract until settlement.

**Timelocked parameters.** Per-netuid and global parameter changes are scheduled first and take effect only after the standard timelock (`CONTRACT_PARAMS_TIMELOCK_MS = 24 h`). Execution is **permissionless** — anyone may call `execute_contract_params_update` / `execute_global_params_update` once the timelock elapses. Ratios are validated: `collateral_ratio` ∈ [100%, 1,000,000%] and strictly > `liquidation_ratio`; `liquidation_ratio` ∈ [100%, 1,000,000%]; `liquidation_fee` ≤ 100%; `transaction_fee` ≤ 100%; `auction_duration_ms` ∈ [60 s, 7 d]; `max_oracle_age_ms` ≠ 0.

**Hotkey migration.** Governance may move all stake to a new hotkey with `set_vault_hotkey(new_hotkey, netuids)` using `move_stake` (func 5), up to 32 netuids per call. The contract remains the coldkey; only the hotkey identity changes. Blocked during active liquidations.

## User flow

1. **Stake alpha** under the vault's hotkey on an approved subnet (if you haven't already). Your stake stays attributed to your coldkey until a deposit pulls it.
2. **Open a vault** — `create_alpha_vault(amount, netuid)` payable with the creation fee (default 0.005 TAO). The contract atomically pulls `amount` of your stake as collateral and returns your `vault_id`. Excess TAO is refunded.
3. **Borrow** — `borrow_token(vault_id, amount)` mints TUSDT to you, provided the vault stays within its borrow limit.
4. **Repay** — `repay_token(vault_id, amount)` burns your TUSDT and reduces debt 1:1. Repaying more than the debt fails with `RepayAmountTooHigh`.
5. **Top up collateral** — `add_alpha_collateral(vault_id, amount)` pulls more stake via the same atomic mechanism.
6. **Release collateral** — `release_alpha_collateral(vault_id, amount, dest_coldkey)` returns stake to your coldkey (`transfer_stake`, func 6) as long as the remaining collateral still covers your debt.
7. **Liquidation** — anyone calls `trigger_liquidation_auction(owner, vault_id)` on an underwater vault → alpha unstaked to TAO, auction created. When the auction is finalized, anyone calls `settle_liquidation_auction(owner, vault_id)`: the winning bid's TUSDT is burned against the debt, the winner gets the TAO collateral minus the transaction fee, and the fee goes to the treasury.
8. **Governance** — `set_approved_netuid`, `set_contract_params` / `set_global_params` (timelocked), `set_vault_hotkey`, `claim_excess_alpha`, `claim_surplus_tusdt`, `transfer_native_to_treasury`, `pause` / `unpause`.

## Key parameters

| Parameter | Meaning | Default | Scale |
|---|---|---|---|
| `collateral_ratio` | Min collateralization required for new borrows / releases | 150% (15,000 bps) | bps (100 = 1%), per-netuid |
| `liquidation_ratio` | Ratio below which the vault is liquidatable | 120% (12,000 bps) | bps, per-netuid |
| `liquidation_fee` | Fee added to debt to form the auction min bid | 11% (1,100 bps) | bps, per-netuid |
| `transaction_fee` | Fee on collateral at auction settlement (TAO side) | 0.3% (30 bps) | bps, global |
| `auction_duration_ms` | Default liquidation auction duration | 3,600,000 (1 h) | ms, [60 s, 7 d], global |
| `max_oracle_age_ms` | Max age of an oracle price before it's stale | 1,800,000 (30 min) | ms, global |
| `vault_creation_fee` | Native TAO fee to open a vault (spam deterrent) | 5,000,000 rao (0.005 TAO) | rao (u64), global, 0 = disabled |
| Param-change timelock | Delay before scheduled param updates execute | 24 h | ms, constant |

**Scale conventions.** All balances are `u64` with 9 decimals: 1 TUSDT = 1e9 units, 1 alpha = 1e9 rao. Oracle prices are 1e18 fixed-point `Ratio`s; the chain alpha price is rao-scaled (1e9). Ratios are configured in basis points (5,000 bps = 50%). Read methods page in chunks of 10 (`PAGE_SIZE`).

## Talks to

- **tusdt-erc20** (spawned child): `mint` on borrow, `burn` on repay and settlement, `transfer` for surplus claims, `balance_of` checks, `set_controller` for upgrades.
- **tusdt-auction** (spawned child): `create_auction` on liquidation trigger, `get_auction` + `transfer_winning_bid` on settlement, `update_governance` on governance hand-off.
- **tusdt-oracle** (spawned child): `get_latest_price` for every pricing read; `update_governance`.
- **tusdt-treasury**: receives creation fees, transaction fees, excess-alpha TAO, and surplus TUSDT.
- **Chain extension** (id `0x1000`): `get_stake_info_for_hotkey_coldkey_netuid` (0), `remove_stake` (2), `move_stake` (5), `transfer_stake` (6), `get_alpha_price` (15), `caller_transfer_stake` (25).

## Errors

All 38 variants are catalogued in the [error reference](../errors/vault-alpha.md). Notable ones:

- `CollateralRatioExceeded` — borrow/release would push the vault past its collateral ratio.
- `NotLiquidatable` — debt is not above the liquidation limit.
- `VaultInLiquidation` — the vault has an active auction; wait for settlement.
- `StakeTransferFailed` — the atomic stake pull failed (insufficient stake, TransferToggle off, below chain minimum, or unsupported call).
- `VaultCreationFeeNotMet` — not enough native TAO sent with `create_alpha_vault`.
- `ActiveLiquidationsExist` — governance op blocked while liquidations are active.
- `OraclePriceStale` / `OraclePriceUnavailable` — pricing is currently impossible; retry after a fresh commit.
