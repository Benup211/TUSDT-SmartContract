# Oracle — the TUSDT/TAO price feed

`tusdt-oracle` publishes the single **TUSDT/TAO price** that the whole protocol relies on: every vault instance and the lending pool read the same latest price for collateral valuation and liquidations. One feed, many consumers — like a **MakerDAO OSM/medianizer** (median-of-reporters with an oracle gap) crossed with **Aave's oracle interface** (a `get_latest_price` contract every money market queries), but with reporter access rooted in **Bittensor subnet neurons** instead of a fixed allowlist.

## How it works

### Rounds and commits

Prices arrive in **rounds**. Reporters submit into the *open* round (`current_round_id`); the **validator** (a governance-set account) commits the round, which records a `PriceData { round_id, price, median_price, reporter_count, committed_at, was_overridden }`, sets it as `latest_price`, and advances to the next round (`finalize_round`). History is kept per round (`get_round_price`, paginated `get_price_history`, page size 10).

### Permissionless reporter model — gated by the subnet, not a list

Any account can try to report: `submit_price(price, metadata)` has no allowlist. Instead, eligibility is verified live against the chain extension (`submit_price`):

1. `metadata.hot_key` must be non-zero, and the `(hotkey, coldkey=caller, netuid)` triplet must have a stake record on the governing subnet — otherwise `NotRegisteredInSubnet`.
2. The neuron's alpha stake must exceed `min_submitter_stake` (default $10\,000\,000\,000$ rao = 10 TAO) — otherwise `InsufficientStake`.
3. The price must be non-zero.

Each coldkey holds **one slot per round**: resubmitting replaces your own submission (`replaced_existing`), up to `MAX_ROUND_SUBMISSIONS` (256) distinct reporters.

### Aggregation: the median

A non-override commit uses the **median** of the round's submissions (`compute_round_median`): odd count → the middle value; even count → the average of the two middle values. `commit_round` requires at least `MIN_REPORTERS` (3) submissions, otherwise `NotEnoughSubmissions`.

### Deviation band and overrides

Each commit (including a validator's override) must stay within the band around the last committed price (`ensure_within_deviation`):

$$|candidate - latest| \le latest \times max\_price\_deviation$$

`max_price_deviation` is a `Ratio` (1e18 inner), default 1 000 bps (10%), settable only by governance. Violations revert with `PriceDeviationExceeded` (there is no latest-price check on the very first commit).

Two override paths exist:

- **Validator override** — `commit_round(Some(price))` commits the supplied price without reporter quorum (`was_overridden = true`) but still inside the deviation band.
- **Governance emergency override** — `commit_round_governance(price)` bypasses **both** quorum and deviation checks (only a zero price is rejected). Governance reaches it through `oracle_commit_round`. This is the deliberate emergency hatch that can push a fresh price when the market has moved — and therefore can drive liquidations immediately.

### Staleness is the consumer's job

The oracle stores `committed_at` (block timestamp, ms) on every `PriceData`, but does **not** reject stale readers itself. The **vault and lending pool** enforce freshness: both validate that the latest price's age is within their `max_oracle_age_ms` (default 1 800 000 ms = 30 minutes) before acting on it.

### Price scale

Prices are `tusdt_primitives::Ratio` — a 1e18 fixed-point value (inner `FixedU128`). A TUSDT price of 250 TAO is stored as $250 \times 10^{18}$. Always scale client-side displays by $10^{18}$.

## User flow

### Report a price (subnet neuron)

1. Ensure your `(coldkey, hotkey)` pair is registered on `netuid` with stake above `min_submitter_stake` (10 TAO by default).
2. Call `submit_price(price, { hot_key, provider })` with a non-zero `Ratio` price; resubmit to update your slot.
3. Wait for the validator to commit the round (≥ 3 reporters for a median commit).

### Commit a round (validator)

1. Call `commit_round(None)` to commit the median, or `commit_round(Some(price))` to override — both must respect the deviation band.

### Read the price (anyone)

Call `get_latest_price()` → `PriceData`; the vault/pool then apply their own `max_oracle_age_ms` staleness check against `committed_at`.

### Emergency price (governance/maintainer)

Call `oracle_commit_round(price)` on governance — the maintainer-only forwarder that lands a deviation-free price instantly.

## Key parameters

| Parameter | Meaning | Default | Scale |
| --- | --- | --- | --- |
| `max_price_deviation` | Max move vs previous committed price | 1 000 bps (10%) | Ratio 1e18 / bps |
| `min_submitter_stake` | Min subnet alpha stake to report | 10 000 000 000 rao (10 TAO) | rao |
| `MIN_REPORTERS` | Min reporters for a non-override commit | 3 | count |
| `MAX_ROUND_SUBMISSIONS` | Distinct reporters per round | 256 | count |
| `PAGE_SIZE` | History page size (`get_price_history`) | 10 | count |
| Price / deviation | All prices and the band | — | Ratio, 1e18 inner |
| Staleness (consumer-side) | Vault/pool `max_oracle_age_ms` | 1 800 000 ms (30 min) | ms |

## Talks to

- **`tusdt-vault-alpha`** — the *controller* (can `update_governance`); reads `get_latest_price` for vault math and staleness checks.
- **`tusdt-lending-pool`** — reads `get_latest_price` for market math and staleness checks.
- **`tusdt-governance`** — holds the `governance` role: `set_validator`, `set_max_price_deviation`, `set_netuid`, `set_min_submitter_stake`, `commit_round_governance`, and `set_controller` (vault upgrades).
- **Bittensor chain extension** — `get_stake_info_for_hotkey_coldkey_netuid` for reporter eligibility.

## Errors

Full catalog: [error reference](../errors/oracle.md). Notable ones: `NotController`, `NotGovernance`, `NotValidator`, `InvalidHotkey`, `NotRegisteredInSubnet`, `InsufficientStake`, `ChainExtensionFailed`, `InvalidPrice`, `NotEnoughSubmissions`, `MedianUnavailable`, `MaxSubmissionsReached`, `PriceDeviationExceeded`, `ArithmeticError`.
