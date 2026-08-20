# Treasury — the protocol reserve

`tusdt-treasury` is the protocol's **reserve and fee-accounting contract** — the TUSDT analogue of **Aave's Collector** or a **MakerDAO Surplus Buffer** with pre-named buckets. Every protocol fee lands here; nothing leaves except through governance. It books both assets (TUSDT and native TAO) into **six named funds** and pays out only when governance authorizes a release — typically via an executed Funding proposal.

## How it works

### The six funds

Incoming value is split across six flat buckets (`Fund` enum + the BPS constants):

| Fund | Share (bps) | Purpose |
| --- | --- | --- |
| `Emergency` | 5 000 (50%) + integer-division remainder | Protocol insurance of last resort; keeps the books balanced |
| `Operation` | 3 000 (30%) | Ongoing development and operations |
| `Insurance` | 1 000 (10%) | Vault loss backstop |
| `Dividend` | 400 (4%) | Staker distributions |
| `Buyback` | 400 (4%) | Market operations |
| `Voting` | 200 (2%) | Governance incentives |

Each fund is tracked separately for TUSDT and native (`funds_tusdt` / `funds_native` mappings, read via `fund_balance_tusdt` / `fund_balance_native`).

### Accounting: `distribute()`

The treasury runs **delta accounting**: the contract's *actual* on-chain balance is reconciled against the *booked* total. `distribute()` is permissionless and idempotent — anyone can call it, and it only books newly arrived value:

$$\text{pending\_tusdt} = token.balance\_of(treasury) - tusdt\_allocated$$
$$\text{pending\_native} = balance(treasury) - native\_allocated$$

Each positive delta is split by `split_delta`:

$$share_f = \left\lfloor delta \times \frac{bps_f}{10\,000} \right\rfloor, \qquad Emergency = delta - \sum_{f \ne Emergency} share_f$$

Because the other five shares are rounded down, crediting the remainder to `Emergency` guarantees the six shares sum exactly to `delta` — the books always reconcile.

### Payouts: `release()`

`release(fund, token_kind, amount, recipient)` is the **only** way value leaves, and it is **governance-only**. It calls `distribute()` first (so freshly arrived fees are booked before a withdrawal), treats `amount == 0` as a no-op, and routes by `token_kind`:

- `TokenKind::Tusdt` → debits `funds_tusdt[fund]`, then `token.transfer(recipient, amount)` on the TUSDT ERC20.
- `TokenKind::Native` → debits `funds_native[fund]`, then a native TAO transfer.

Debiting more than the fund's booked balance reverts with `InsufficientFundBalance`.

### Where the money comes from

- **Vault creation fee** — native TAO, transferred to the treasury when a vault opens.
- **Transaction fees** — native TAO, charged on the collateral only at liquidation auction
  settlement (`transfer_transaction_fee_to_treasury`, default 0.3%). Vault borrows and repays
  carry no fee.
- **Liquidation surplus** — TUSDT claimed from settled vault liquidations.
- **Excess alpha** — governance's `vault_claim_excess_alpha` unwinds excess staked alpha into TAO sent here.
- **Lending pool** — the reserve slice of interest (`reserve_accrued`) and the performance fee (default 25%) accrue as pool surplus; governance sweeps it via `pool_claim_surplus_tusdt` / `pool_transfer_native_to_treasury`.

## User flow

### Fees in, books updated

1. Protocol contracts transfer TUSDT and/or native TAO to the treasury account (fee paths above).
2. Anyone calls `distribute()`; the deltas are booked into the six funds (remainder → `Emergency`), emitting `FundsDistributed`.

### Governance spends

1. A council member submits a **Funding** proposal on governance with `Fund`, `TokenKind`, `amount`, and `recipient`.
2. After vote → finalize → execute, governance calls `treasury.release(...)`; the treasury books the debit, performs the transfer, and emits `FundReleased`.

### Direct release

The governance contract itself can call `release(fund, token_kind, amount, recipient)` at any time — the Funding-proposal path is the community-visible route, not the only one.

## Key parameters

| Parameter | Meaning | Default | Scale |
| --- | --- | --- | --- |
| `EMERGENCY_BPS` … `VOTING_BPS` | Fund shares | 5 000 / 3 000 / 1 000 / 400 / 400 / 200 | bps, /10 000 |
| `TOTAL_BPS` | Shares must sum to this | 10 000 | bps |
| Balances | All fund and allocated balances | — | u64, 9 decimals (1 TUSDT/TAO = 1e9) |

## Talks to

- **`tusdt-governance`** — the only `release` caller and the only account that can `set_governance`; receives governance's Funding-proposal payouts.
- **`tusdt-erc20`** — `balance_of` for delta accounting, `transfer` for TUSDT releases.
- **`tusdt-vault-alpha`** / **`tusdt-lending-pool`** — senders of fees and sweeps (creation/transaction/liquidation fees, excess alpha, pool reserve + performance surplus).

## Errors

Full catalog: [error reference](../errors/treasury.md). The four variants: `NotGovernance`, `InsufficientFundBalance`, `TransferFailed`, `ArithmeticError`.
