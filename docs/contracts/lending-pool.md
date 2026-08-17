# tusdt-lending-pool — Lending Pool

The lending pool is the protocol's money market: suppliers deposit **TAO** or **TUSDT** and earn
variable interest, borrowers lock subnet **alpha** stake as collateral and borrow against it. Think
of it as Aave/Compound on Bittensor — utilization-based interest rates, scaled debt, lToken receipt
tokens, health-factor risk checks — with two twists: your alpha collateral keeps staking on its
subnet while it backs your loan, 75% of that staking yield is credited back to borrowers, and
idle supplied TAO is staked to the Bittensor root subnet (1:1 TAO↔alpha) instead of sitting
unproductive in the pool's balance.
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
  the contract's native balance plus any root-subnet stake (`balance + staked_tao`, see Idle
  TAO root-subnet staking below) or its own TUSDT balance (`market_cash`, `rates.rs`).
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
`claim_reserve(market_id)` (capped at the market's free balance, never root stake).

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

### Idle TAO root-subnet staking

Supplied TAO that nobody is borrowing just sits in the pool's balance, earning nothing for
anyone. Root-subnet staking puts it to work: the pool stakes its **excess free TAO** into the
Bittensor **root subnet (netuid 0)** through the chain extension (`add_stake`, func 1),
acting as its own coldkey. Root is the ideal destination for idle TAO because it is the only
subnet where stake is **1:1 TAO↔alpha** and where staking/unstaking costs **zero fee and
zero slippage** and — today — settles in the **same extrinsic** (`remove_stake`, func 2).
Lido-style, minus the queue and the stTAO token: the stake is still the pool's own TAO, just
parked where it earns.

**Two sleeves.** The pool's TAO lives in one of two sleeves:

- **Free sleeve** — the contract's native balance. `withdraw_tao`, `borrow_tao`, and capped
  outflows draw from it directly, so it always keeps at least `stake_buffer` native TAO for
  instant withdrawals.
- **Root sleeve** — TAO staked on netuid 0, tracked by `staked_tao`. It earns root yield but
  is one unstake away from spendable, so it only ever receives excess.

The split is governed by three Balance-scale parameters:

| Parameter | Meaning | Default |
|---|---|---|
| `stake_buffer` | Native TAO kept in the free sleeve | `1_000_000_000` (1 TAO) |
| `sweep_threshold` | Free balance must be this far above the buffer before a sweep moves anything | `0` |
| `stake_floor` | Minimum root-stake amount (pallet minimum) and dust cutoff | `2_000_000` (0.002 TAO) |

Hard invariants: `stake_floor ≥ 2_000_000` (the chain's `add_stake` minimum) and
`stake_buffer ≥ stake_floor`. Sweep candidates below `stake_floor` are skipped, and no
partial unstake ever strands a remainder below the floor.

**Governance config.** One governance-only message steers the whole feature:

`set_root_stake_config(root_hotkey, staking_enabled, stake_buffer, sweep_threshold, stake_floor)`

- `root_hotkey` — the hotkey under which the pool's root stake sits (the contract is the
  coldkey; only this hotkey's position counts).
- `staking_enabled` — the master switch, **default `false`**. A fresh deployment or upgrade
  therefore starts with staking off: all TAO stays in the free sleeve and behavior is
  identical to the pre-staking contract until governance configures and enables it.
- `stake_buffer` / `sweep_threshold` / `stake_floor` — validated against the invariants
  above.

**Rotating `root_hotkey` while `staked_tao > 0` first fully unstakes** — no TAO is ever left
registered under a retired hotkey. Every change emits `RootStakeConfigUpdated{...}`; current
state is readable via `get_root_stake_config()` and `get_tao_staked()`.

**Lifecycle — sweeps.** Sweeping is opportunistic and never blocks the caller. Whenever a
state-changing call leaves the free sleeve over-provisioned,

```
if free_balance > stake_buffer + sweep_threshold:   # trigger
    stake free_balance − stake_buffer to root       # minus any sub-floor dust
```

the excess above `stake_buffer` is staked via `add_stake` in the same extrinsic. Sweeps
piggyback on **supply and repay** — the calls most likely to grow the free balance — and
anyone can also trigger one through the permissionless keeper entry point `sweep()`,
**rate-limited to once per block** by `last_sweep_block`. Each sweep bumps `staked_tao` and
emits `StakedIdleTao{amount}`.

**Lifecycle — top-ups.** When a `withdraw_tao` or `borrow_tao` would exceed the free sleeve,
the pool reads actual root availability (`get_stake_availability`, func 36) and
**synchronously unstakes** the shortfall before serving the request:

```
if free_balance < needed:  unstake min(needed − free_balance, staked_tao)
```

Root's instant, 1:1 unstake adds zero latency and zero slippage today. A partial unstake
that would strand `< stake_floor` in the root sleeve instead empties it entirely (full
exit). Every top-up reduces `staked_tao` and emits `UnstakedIdleTao{amount}`.

**market_cash counts both sleeves.** The TAO market's cash is the union of the two sleeves,
not the bare balance:

```
market_cash(0) = balance + staked_tao
```

Sweeps and top-ups are therefore **balance-neutral** to the market: moving X TAO between
sleeves leaves cash unchanged, so utilization (`U = total_debt / (total_debt + cash)`),
total supplied, and the lTAO exchange rate never jump when stake moves. If cash were the
bare balance, every sweep would masquerade as a liquidity drain and every top-up as a mint.
The union accounting is also what keeps borrows and withdrawals servable up to `staked_tao`
beyond the free balance (via the top-up path above). TUSDT (market 1) is untouched.

**Outflow caps.** Two outflows are capped so neither can drain the free sleeve below its
buffer or force a root unstake:

- `claim_reserve(market_id)` — the interest reserve can be claimed only up to the market's
  **free balance**; it never reaches into root stake.
- `transfer_native_to_treasury` — surplus TAO sweeps to the treasury are capped at
  `balance − stake_buffer`, preserving the withdrawal buffer and leaving the root sleeve
  alone.

**Risks and mitigations.**

- **`RootStakeUnlockInterval` (the live hold lever).** Root unstaking is instant today only
  because the unstake-hold interval is currently 0/disabled — a live Subtensor governance
  parameter that can be switched on at any time. Every `add_stake`/`remove_stake` re-stamps
  the age of the pool's root position, so under an enabled hold the most recently swept TAO
  is the least liquid, and top-ups would inherit the full interval. Mitigation:
  `staking_enabled` is the kill switch — governance can stop new sweeps and unwind before
  (or while) the lever is live.
- **Basket-seed maintenance windows.** Root-subnet stake operations can be paused or delayed
  during maintenance windows (e.g. basket-seed maintenance). The pool never assumes root
  liquidity is available exactly when needed: sweeps simply skip, and a withdrawal or borrow
  that needs a top-up during a window fails the normal liquidity check rather than breaking
  an invariant.
- **Dust rules (floor, full exit).** Sub-floor amounts are never staked, and partial
  unstakes never strand a sub-floor remainder — no dust can be trapped in either sleeve.
- **Hotkey rotation.** Replacing `root_hotkey` with stake outstanding first fully unstakes,
  so TAO can never be orphaned under a retired hotkey.
- **Upgrade defaults.** `staking_enabled = false` on every fresh state, so upgrades preserve
  the pre-staking behavior exactly until governance opts in.

**Phase 2 — dividend harvesting (optional, future).** The root subnet pays TAO dividends to
the staking hotkey. A later phase may add an optional harvester that sweeps these dividends
to the **treasury** — deliberately **not** into the lTAO exchange rate, so lToken value stays
determined purely by borrow interest and the harvester can never inflate withdrawal rights.

### Timelocked parameter updates

Interest-rate, alpha, and global parameters can't change instantly. `set_market_params`,
`set_alpha_params`, and `set_global_params` (maintainer-only) validate and queue a pending
update; anyone can execute it via the matching `execute_*_update` after the delay
(`PARAMS_TIMELOCK_MS = 86_400_000 ms = 24 hours`, `params.rs`), and the maintainer can cancel
beforehand. Execution before the delay reverts with `ParamsUpdateTimelockActive`.

## User flow

1. **Supply TAO / TUSDT** — `supply_tao(amount)` (payable; excess TAO refunded) or
   `supply_tusdt(amount)` (pulls TUSDT via `transfer_from`). You receive lTAO / lTUSDT at the
   current exchange rate. Subject to supply caps (`SupplyCapExceeded`). Excess free TAO is
   opportunistically swept to the root subnet in the same call (see Idle TAO root-subnet
   staking).
2. **Deposit alpha collateral** — `deposit_alpha(netuid, amount)` atomically pulls your staked
   alpha from under the pool's hotkey via chain extension func 25 (caller-forwarded) and books
   `alpha_principal`. This gives you borrowing power.
3. **Borrow** — `borrow_tao(amount)` or `borrow_tusdt(amount)` against your alpha collateral, up
   to `min_collateral_factor · collateral − debt` (`BorrowHealthExceeded`), limited by market
   cash (`LiquidityInsufficient`) and borrow caps (`BorrowCapExceeded`).
4. **Repay** — `repay_tao(amount)` (payable) or `repay_tusdt(amount)` (`transfer_from`).
   Repayment clamps to your current debt; repaid assets stay in the pool as liquidity (and
   may be swept to the root subnet).
5. **Withdraw** — `withdraw_tao / withdraw_tusdt(ltoken_amount)` burn receipts for underlying
   (liquidity permitting; a TAO shortfall is topped up synchronously from root stake);
   `withdraw_alpha(netuid, amount, dest_coldkey)` returns alpha stake
   via func 6, but only while you stay healthy (`HealthFactorBelowThreshold`).
6. **Claim alpha yield** — `claim_alpha_yield(netuid)`: 25% of excess staking yield goes to the
   treasury, 75% raises your collateral's yield index. Also `claim_reserve(market_id)` for the
   interest reserve.
7. **Liquidate** — when a borrower's health factor is below 1.0, anyone can call `liquidate` to
   repay up to 50% of their debt and seize alpha collateral at a 5% bonus.
8. **Root-subnet staking (keeper)** — anyone can call `sweep()` (rate-limited to once per
   block) to stake excess idle TAO into the root subnet; governance configures and enables
   the feature via `set_root_stake_config`.

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
| `staking_enabled` | Root-subnet staking master switch | false | bool |
| `stake_buffer` | Free-sleeve TAO kept for instant withdrawals | 1_000_000_000 (1 TAO) | Balance |
| `sweep_threshold` | Free balance above buffer required before a sweep | 0 | Balance |
| `stake_floor` | Minimum root stake / dust cutoff | 2_000_000 (0.002 TAO) | Balance |
| `last_sweep_block` | Rate-limits keeper `sweep()` to one per block | — | block |
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
- **Root subnet (netuid 0)** — holds the pool's idle-TAO stake (`add_stake` (1),
  `remove_stake` (2)); the contract is its own coldkey, staking under `root_hotkey`.
- **Chain extension** — `get_alpha_price` (15), `caller_transfer_stake` (25),
  `get_stake_availability` (36, alpha yield and root top-ups), `move_stake` (5, hotkey
  migration), `transfer_stake` (6), `add_stake` (1, root-subnet sweeps), `remove_stake`
  (2, alpha yield unstake and root top-ups).

## Errors

All 43 fieldless error variants are catalogued with guidance in the
[error reference](../errors/lending-pool.md). Notable ones: `BorrowHealthExceeded`,
`LiquidityInsufficient`, `HealthFactorBelowThreshold`, `NotLiquidatable`,
`ParamsUpdateTimelockActive`, `SupplyCapExceeded`.

Root-subnet staking adds **no new error variants** — invalid `set_root_stake_config`
parameters and sweep/top-up failures reuse the existing catalogue.
