# tUSDT token — the stablecoin

`tusdt-erc20` is **TUSDT**, the protocol's stablecoin — the TUSDT analogue of **DAI/USDC**. It is a PSP22-style fungible token (`transfer`, `transfer_from`, `approve`, `increase_allowance`, `decrease_allowance`, `Transfer`/`Approval` events) with **9 decimals**: 1 TUSDT = $10^9$ base units, and every balance/total is a u64 in those units.

## How it works

### Multi-minter model

Supply is controlled by a **minters set** — only member accounts can `mint` or `burn`:

- The **controller** is the contract's admin, set at construction (`new`) and also seated as the initial minter.
- `add_minter` / `remove_minter` (controller-only) manage the set; `is_minter` reports membership.
- `set_controller(new)` (controller-only) swaps the role **and** moves the minter seat with it — the old controller is removed from minters, the new one added.

In practice the controller hand-off goes: deployer → vault → (on upgrade, via governance's `vault_set_token_controller`) new vault. The **vault** is the primary minter — it mints TUSDT when borrowers draw and burns it on repayment/settlement. The controller may seat additional minters as the protocol grows.

### Transfers and allowances

`transfer` / `transfer_from` share one core (`transfer_from_to`); `transfer_from` also debits the caller's allowance. `increase_allowance` is saturating (over-the-top is clamped), while `decrease_allowance` reverts with `InsufficientAllowance` if you try to go below zero — the standard anti-front-running shapes.

## User flow

1. **Get TUSDT** — borrow against alpha in a vault (the vault mints TUSDT to you) or buy on a market.
2. **Send it** — `transfer(to, amount)`; or approve a spender (`approve`/`increase_allowance`) and let it `transfer_from`.
3. **Check balances** — `balance_of`, `allowance`, `total_supply`.

## Key parameters

| Parameter | Meaning | Default | Scale |
| --- | --- | --- | --- |
| Decimals | Unit scale (1 TUSDT) | 9 | $10^9$ base units |
| Balances / supply | All amounts | — | u64, 9 decimals |
| Minters set | Who may `mint`/`burn` | controller + any added | accounts |

## Talks to

- **`tusdt-vault-alpha`** — controller and primary minter (mint on borrow, burn on repay).
- **`tusdt-treasury`** — holds/releases TUSDT; reads `balance_of`, calls `transfer`.
- **`tusdt-governance`** — indirectly moves the controller via `vault_set_token_controller` on vault upgrades.

## Errors

Full catalog: [error reference](../errors/erc20.md). The five variants: `InsufficientBalance`, `InsufficientAllowance`, `NotMinter`, `NotController`, `ArithmeticError`.
