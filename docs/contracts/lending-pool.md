# tusdt-lending-pool — Lending Pool

The lending pool is the protocol's money market: suppliers deposit **TAO** or **TUSDT** and earn
variable interest, borrowers lock subnet **alpha** stake as collateral and borrow against it. Think
of it as Aave/Compound on Bittensor — utilization-based interest rates, scaled debt, lToken receipt
tokens, health-factor risk checks — with one twist: your alpha collateral keeps staking on its
subnet while it backs your loan, and 75% of that staking yield is credited back to borrowers.
Contract: `contracts/tusdt-lending-pool/lib.rs` (package `tusdt-lending-pool`, artifact
`tusdt_lending_pool`).

## Scale conventions

Read the formulas below with these units:

| Quantity | Scale | Example |
|---|---|---|
| Balances (`Balance = u64`) | 9 decimals | `1_000_000_000` = 1 TUSDT |
| Config parameters | basis points | `5000` = 50% |
| Internal ratios (`Ratio`) | 1e18 | `10^18` = 1.0 |
| Alpha price (chain extension func 15) | rao per alpha, 1e9 | `rao/1e9` = TAO per alpha |
| Oracle price | 1e18 | TUSDT per TAO |
| Time base | 8760 h/year | 365 days |

## How it works

### Markets

The pool runs two kinds of markets, keyed by `market_id`:

- **0 = TAO** and **1 = TUSDT** — supply-and-borrow markets with interest accrual. Market cash is
  the contract's native balance (TAO) or its own TUSDT balance (`market_cash`, `rates.rs`).
- **2, 3, … = alpha collateral markets** — one per subnet approved via `set_approved_netuid`.
  Collateral-only: no lToken, no debt, and interest accrual is a no-op for ids ≥ 2
  (`accrue_interest`, `rates.rs`).

### Utilization and the borrow-rate curve

Utilization measures how much of a market's liquidity is lent out:

```
U = total_debt / (total_debt + cash)
```

The annual borrow rate follows a two-zone curve (`compute_borrow_rate`, `rates.rs`):

```
if U ≤ U_opt:  r = base + slope1 · (U / U_opt)
if U > U_opt:  r = base + slope1 + slope2 · (U − U_opt) / (1 − U_opt)
```

Defaults (`default_*_interest_params`, `params.rs`) — verified in tests (`borrow_rate_in_zone_2`):

| Market | base | slope1 | slope2 | optimal | reserve_factor |
|---|---|---|---|---|---|
| TAO (0) | 0% | 4% | 96% | 80% | 20% |
| TUSDT (1) | 0% | 3% | 97% | 80% | 20% |

Worked example (TAO market, `U = 90%`): `r = 4% + 96% · (0.9−0.8)/(1−0.8) = 52%` per year.
At full utilization the TAO rate is `0 + 4% + 96% = 100%`. Validation caps
`slope1 + slope2 ≤ 100%` and the total maximum rate at 100% (`validate_interest_params`).

### Interest accrual: hourly discrete compounding

Interest accrues lazily on every state-changing call to a supply/borrow market
(`accrue_interest`, `rates.rs`):

```
dt_hours = dt_ms / 3_600_000          # sub-hour remainder is dropped
borrow_growth = (1 + r_annual / 8760)^dt_hours
supply_growth = (1 + s_annual / 8760)^dt_hours
```

Then the market accumulators advance:

```
total_debt    ← borrow_growth · total_debt
borrow_index  ← borrow_growth · borrow_index
exchange_rate ← supply_growth · exchange_rate
```

Rules: no accrual while `total_debt == 0`, and nothing accrues until a full hour has passed
(`dt_hours == 0`) — the partial hour is discarded (timestamp still advances). Alpha markets
(ids ≥ 2) skip accrual entirely.

### Supply rate and the reserve

Suppliers receive the borrower interest minus the protocol's reserve slice
(`accrue_interest`, `rates.rs`):

```
s_annual = r_annual · U · (1 − reserve_factor)        # default reserve_factor = 20%
reserve_delta = debt_interest − supply_interest       # → reserve_accrued
```

The reserve accumulates per market and anyone can send it to the treasury via permissionless
`claim_reserve(market_id)` (capped at available cash).

### Scaled debt

Per-user debt is stored as a fixed number that grows only when the user acts; the market-wide
`borrow_index` (starts at `1e18`) does the compounding:

```
actual_debt = scaled_debt · borrow_index / 1e18
```

Borrowing adds `scaled += amount / borrow_index`; repaying subtracts
`scaled −= repay_amount / borrow_index`. Repayments clamp:
`repay_amount = min(amount, actual_debt)` — you can never overpay (excess TAO is refunded; TUSDT
overpayment is simply not pulled). A parallel `debt_principal` mapping tracks principal so
`interest = debt − principal` is always readable (`get_user_debt_details`).

### lTokens (Compound-style receipts)

Each supply market has a child ERC-20 receipt token (lTAO, lTUSDT), spawned from the
`tusdt-erc20` code hash at construction. The exchange rate starts at 1.0 and grows at the supply
rate — your token balance never changes, its worth does:

```
supply:   ltoken_minted = amount · ltoken_total_supply / total_supplied   # 1:1 at genesis
withdraw: underlying    = ltoken_burned  · total_supplied / ltoken_supply
underlying = ltoken_balance · exchange_rate
```

Amounts that round to zero revert with `MintBelowPrecision`.

### Alpha collateral value

Your collateral is **effective alpha** — principal grown by the per-netuid yield index:

```
effective = alpha_principal · yield_index / 1e18
```

Each approved subnet has a price `alpha_price_rao` (rao per alpha, from chain extension func 15),
combined with the oracle's TUSDT/TAO price (`collateral_price`, `risk.rs`):

```
collateral_price = oracle_tusdt_per_tao · (alpha_price_rao / 1e9)      # TUSDT per alpha unit
collateral_value_tusdt = collateral_price · effective                  # 9-decimal TUSDT units
```

Expanded, each scale is explicit:
`value = (alpha_principal · yield_index / 1e18) · (alpha_price_rao / 1e9) · oracle_tusdt_per_tao`.

### Borrow capacity and health factor

Borrowing power uses the **minimum** collateral factor across the alpha markets you hold;
liquidation risk uses the **maximum** liquidation threshold (`risk.rs`):

```
borrow_capacity_tusdt = min_collateral_factor · collateral_value − debt_value
health_factor         = max_liquidation_threshold · collateral_value / debt_value
```

`debt_value` is your TUSDT debt plus TAO debt converted at the oracle price
(`get_debt_value_tusdt`). Health factor is `None` when you have no debt and `0` when you have no
collateral; a position is **liquidatable when health < 1.0** (`is_liquidatable`). `borrow_tao` /
`borrow_tusdt` reject amounts above your capacity (`BorrowHealthExceeded`); `withdraw_alpha`
simulates the post-withdrawal state and rejects if it would push you underwater
(`HealthFactorBelowThreshold`). If you hold no alpha, the fallback threshold is 60%.

### Liquidation

Liquidation is permissionless: `liquidate(borrower, debt_market, debt_to_cover,
collateral_netuid)`. The liquidator repays part of the borrower's debt and seizes alpha
collateral at a discount:

```
max_cover_tusdt  = close_factor · borrower_total_debt_value        # default close_factor = 50%
actual_debt_units = min(debt_to_cover, max_cover, debt in that market)
seizure_value_tusdt = cover_value · (1 + liquidation_bonus)        # default bonus = 5%
alpha_seized        = seizure_value / collateral_price
principal_seized    = alpha_seized / yield_index
```

The borrower's scaled debt and alpha principal are reduced by the seized amounts, the liquidator
pays the debt (native TAO via `transferred_value`, or TUSDT via `transfer_from`), and receives
the alpha stake via chain extension `transfer_stake` (func 6). Seizing more principal than the
borrower holds reverts with `CollateralAwardExceedsPosition`.

### Alpha yield claim

Alpha collateral keeps earning staking yield. Anyone can call `claim_alpha_yield(netuid)`:

```
booked  = yield_index · netuid_total_collateral / 1e18
excess  = available_stake (func 36) − booked          # no-op if ≤ 0
fee     = performance_fee · excess                    # default 25% → unstaked (func 2) → treasury
credited = excess − fee                               # 75% stays staked
new_index = (booked + credited) / netuid_total_collateral
```

The credited 75% raises the yield index, proportionally increasing every borrower's effective
collateral (and therefore their borrow capacity) on that subnet.

### Timelocked parameter updates

Interest-rate, alpha, and global parameters can't change instantly. `set_market_params`,
`set_alpha_params`, and `set_global_params` (maintainer-only) validate and queue a pending
update; anyone can execute it via the matching `execute_*_update` after the delay
(`PARAMS_TIMELOCK_MS = 86_400_000 ms = 24 hours`, `params.rs`), and the maintainer can cancel
beforehand. Execution before the delay reverts with `ParamsUpdateTimelockActive`.

## User flow

1. **Supply TAO / TUSDT** — `supply_tao(amount)` (payable; excess TAO refunded) or
   `supply_tusdt(amount)` (pulls TUSDT via `transfer_from`). You receive lTAO / lTUSDT at the
   current exchange rate. Subject to supply caps (`SupplyCapExceeded`).
2. **Deposit alpha collateral** — `deposit_alpha(netuid, amount)` atomically pulls your staked
   alpha from under the pool's hotkey via chain extension func 25 (caller-forwarded) and books
   `alpha_principal`. This gives you borrowing power.
3. **Borrow** — `borrow_tao(amount)` or `borrow_tusdt(amount)` against your alpha collateral, up
   to `min_collateral_factor · collateral − debt` (`BorrowHealthExceeded`), limited by market
   cash (`LiquidityInsufficient`) and borrow caps (`BorrowCapExceeded`).
4. **Repay** — `repay_tao(amount)` (payable) or `repay_tusdt(amount)` (`transfer_from`).
   Repayment clamps to your current debt; repaid assets stay in the pool as liquidity.
5. **Withdraw** — `withdraw_tao / withdraw_tusdt(ltoken_amount)` burn receipts for underlying
   (liquidity permitting); `withdraw_alpha(netuid, amount, dest_coldkey)` returns alpha stake
   via func 6, but only while you stay healthy (`HealthFactorBelowThreshold`).
6. **Claim alpha yield** — `claim_alpha_yield(netuid)`: 25% of excess staking yield goes to the
   treasury, 75% raises your collateral's yield index. Also `claim_reserve(market_id)` for the
   interest reserve.
7. **Liquidate** — when a borrower's health factor is below 1.0, anyone can call `liquidate` to
   repay up to 50% of their debt and seize alpha collateral at a 5% bonus.

## Key parameters

| Parameter | Meaning | Default | Scale |
|---|---|---|---|
| `base_rate` | Borrow rate at 0% utilization | 0 / 0 (TAO/TUSDT) | bps |
| `slope1` | Rate slope below optimal utilization | 400 / 300 | bps |
| `slope2` | Rate slope above optimal utilization | 9600 / 9700 | bps |
| `optimal_utilization` | Kink of the rate curve | 8000 (80%) | bps |
| `reserve_factor` | Protocol share of borrower interest | 2000 (20%) | bps |
| `collateral_factor` | Max borrow power per alpha unit | 5000 (50%) | bps |
| `liquidation_threshold` | Health-factor denominator per netuid | 6000 (60%) | bps |
| `liquidation_bonus` | Discount liquidators receive | 500 (5%) | bps |
| `close_factor` | Max share of debt per liquidation | 5000 (50%) | bps |
| `performance_fee` | Alpha-yield cut to the treasury | 2500 (25%) | bps |
| `max_oracle_age_ms` | Max age of an acceptable price | 1_800_000 (30 min) | ms |
| `supply_cap_tao / _tusdt` | Max supplied per market (0 = unlimited) | 0 | Balance |
| `borrow_cap_tao / _tusdt` | Max debt per market (0 = unlimited) | 0 | Balance |
| `PARAMS_TIMELOCK_MS` | Param-update delay | 86_400_000 (24 h) | ms |
| `borrow_index` | Scaled-debt accumulator | starts 1e18 | Ratio |
| `exchange_rate` | lToken redemption rate | starts 1e18 | Ratio |
| `netuid_yield_index` | Effective-collateral multiplier per netuid | starts 1e18 | Ratio |

## Talks to

- **lTokens (spawned children)** — lTAO / lTUSDT `tusdt-erc20` instances, minted/burned on
  supply/withdraw (`mint`, `burn`, `total_supply`).
- **TUSDT token** — `transfer_from` on supply/repay/liquidation; `transfer` on borrow/withdraw.
- **Oracle** — `get_latest_price()` for TUSDT/TAO, freshness-checked against
  `max_oracle_age_ms` (`OraclePriceStale`).
- **Treasury** — receives the interest reserve, the alpha performance fee, and surplus sweeps.
- **Chain extension** — `get_alpha_price` (15), `caller_transfer_stake` (25),
  `get_stake_availability` (36), `move_stake` (5, hotkey migration), `transfer_stake` (6),
  `remove_stake` (2, alpha yield unstake).

## Errors

All 43 fieldless error variants are catalogued with guidance in the
[error reference](../errors/lending-pool.md). Notable ones: `BorrowHealthExceeded`,
`LiquidityInsufficient`, `HealthFactorBelowThreshold`, `NotLiquidatable`,
`ParamsUpdateTimelockActive`, `SupplyCapExceeded`.
