# TUSDT Contracts Workspace

Ink! contracts for a collateralized TUSDT system with vault borrowing, interest accrual, and
liquidation auctions, plus on-chain governance and a treasury that books protocol fees.

The contracts split into two layers:

- **Protocol** — `tusdt-erc20` (token), `tusdt-vault` (CDP borrowing/liquidation), `tusdt-auction`
  (liquidation auctions), `tusdt-oracle` (collateral pricing). The vault owns the token/auction/oracle
  instances it creates.
- **Governance & treasury** — `tusdt-governance` (token-holder proposals plus a maintainer/council
  authority that steers the protocol contracts) and `tusdt-treasury` (per-fund accounting for fees,
  released only by governance).

## Prerequisites

- Rust stable toolchain
- `wasm32-unknown-unknown` target
- `cargo-contract`
- Node.js + Yarn (for the isolated contract tooling under `tools/`)
- A Contracts-enabled Substrate node (local or remote)

```bash
rustup target add wasm32-unknown-unknown
cargo install --locked cargo-contract
```

## Build

Build all crates:

```bash
cargo check
```

Build contract artifacts (`.contract`, `.wasm`, metadata):

```bash
cargo contract build --manifest-path contracts/tusdt-erc20/Cargo.toml --release
cargo contract build --manifest-path contracts/tusdt-auction/Cargo.toml --release
cargo contract build --manifest-path contracts/tusdt-oracle/Cargo.toml --release
cargo contract build --manifest-path contracts/tusdt-vault/Cargo.toml --release
cargo contract build --manifest-path contracts/tusdt-treasury/Cargo.toml --release
cargo contract build --manifest-path contracts/tusdt-governance/Cargo.toml --release
```

Artifacts are produced in `target/ink/`.

## Contract Tooling (`tools/`)

Shared deployment scripts and on-chain tests live in an isolated TypeScript subproject under `tools/`.
The current iteration exposes upload support for `erc20`, `auction`, `oracle`, and `vault`, plus a single `vault` deployment entrypoint that instantiates the whole runtime flow. The `treasury` and `governance` contracts are not yet scripted here — build and deploy them with `cargo contract` directly (see below).

Setup:

```bash
cd tools
yarn install
cp .env.example .env
```

Default `.env` values target a local dev node:

- `WS_URL=ws://127.0.0.1:9944`

Scripts and tests use the standard local dev accounts (`//Alice`, `//Bob`, `//Charlie`, `//Dave`, `//Eve`, `//Ferdie`) from the shared dev-account helper.
When needed, you can override the selected account via SURI environment variables such as `CONTRACT_UPLOADER=//Alice`, `CONTRACT_DEPLOYER=//Alice`.

Useful commands:

```bash
cd tools
yarn build:erc20-artifacts
yarn build:auction-artifacts
yarn build:oracle-artifacts
yarn build:vault-artifacts
yarn erc20:upload
yarn auction:upload
yarn oracle:upload
yarn vault:upload
yarn vault:deploy --token-code-hash <TOKEN_CODE_HASH> --auction-code-hash <AUCTION_CODE_HASH> --oracle-code-hash <ORACLE_CODE_HASH>
yarn test:oracle
```

## Deployment (Recommended Order)

`tusdt-vault::new` takes a `treasury` address plus the **code hash** of the token, auction, and
oracle contracts, then instantiates all three internally. Because the vault *creates* the TUSDT
token, the token address is not known until the vault exists — and the treasury needs that token
address — so the treasury is deployed after the vault and wired in via `update_treasury`.

1. Upload ERC20 code (`tusdt-erc20`) and capture code hash.
2. Upload Auction code (`tusdt-auction`) and capture code hash.
3. Upload Oracle code (`tusdt-oracle`) and capture code hash.
4. Instantiate Vault (`tusdt-vault::new`) with:
   - `treasury` — a placeholder address for now (e.g. the deployer); reassigned in step 7.
   - `token_code_hash`
   - `auction_code_hash`
   - `oracle_code_hash`

   The deployer becomes the initial governance of the vault, and (via instantiation) of the auction
   and oracle.
5. Read the token / auction / oracle addresses from the vault (`get_token_address`,
   `get_auction_address`, `get_oracle_address`).
6. Instantiate Treasury (`tusdt-treasury::new`) with the vault's TUSDT token address. The deployer
   becomes the treasury's initial governance.
7. Point the vault's fee recipient at the treasury: `tusdt-vault::update_treasury(treasury)`.
8. Instantiate Governance (`tusdt-governance::new`) with `treasury`, `vault`, `auction`, `oracle`
   addresses and the initial `maintainer` (typically the subnet owner).
9. Hand control to the governance contract:
   - `tusdt-treasury::set_governance(governance)` — only governance may release funds afterward.
   - `tusdt-vault::update_governance(governance)` — propagates the role to the auction and oracle too.
10. Seat the council: as the maintainer, call `tusdt-governance::set_council([c1..c5])`.

After step 10 the protocol contracts are steered exclusively by the governance contract, and within
governance the maintainer/council split (see [Governance & Treasury](#governance--treasury)) applies.

Example CLI for the protocol layer (adjust URL/account):

```bash
cargo contract upload \
  --manifest-path contracts/tusdt-erc20/Cargo.toml \
  --suri //Alice --url ws://127.0.0.1:9944

cargo contract upload \
  --manifest-path contracts/tusdt-auction/Cargo.toml \
  --suri //Alice --url ws://127.0.0.1:9944

cargo contract upload \
  --manifest-path contracts/tusdt-oracle/Cargo.toml \
  --suri //Alice --url ws://127.0.0.1:9944

cargo contract instantiate \
  --manifest-path contracts/tusdt-vault/Cargo.toml \
  --constructor new \
  --args <TREASURY_OR_PLACEHOLDER> <ERC20_CODE_HASH> <AUCTION_CODE_HASH> <ORACLE_CODE_HASH> \
  --suri //Alice --url ws://127.0.0.1:9944
```

Then deploy the governance layer and wire the roles:

```bash
cargo contract instantiate \
  --manifest-path contracts/tusdt-treasury/Cargo.toml \
  --constructor new \
  --args <TUSDT_TOKEN_ADDRESS> \
  --suri //Alice --url ws://127.0.0.1:9944

cargo contract instantiate \
  --manifest-path contracts/tusdt-governance/Cargo.toml \
  --constructor new \
  --args <TREASURY_ADDRESS> <VAULT_ADDRESS> <AUCTION_ADDRESS> <ORACLE_ADDRESS> <MAINTAINER> \
  --suri //Alice --url ws://127.0.0.1:9944
```

Prefer the `tools/` workflow above instead of using `cargo contract` for upload/deploy operations
where a TS script already exists. The current e2e test suite is intentionally oracle-only, the only
deployment script entrypoint is `vault:deploy`, and the treasury/governance contracts have no TS
scripts yet — deploy and wire them with `cargo contract` as shown.

## Working Flow

### 1) Vault lifecycle

1. User creates vault with native collateral: `create_vault` (payable).
2. User adds collateral: `add_collateral` (payable).
3. User borrows token: `borrow_token`.
4. User repays token: `repay_token`.
5. Anyone can trigger vault debt accrual: `accrue_interest(owner, vault_id)`.
6. User releases collateral (only when debt is zero): `release_collateral`.

### 2) Interest model

- Accrual is hour-based.
- The configured `interest_rate` is an APR-style annual rate, not a simple yearly charge.
- Growth model uses discrete hourly compounding from that annual rate.
- At the default `5%` APR, the effective annualized cost is approximately `5.13%` APY under hourly compounding.
- Implementation compounds by elapsed full hours and advances `last_interest_accrued_at` to the last fully accrued hour.

### 3) Liquidation flow

1. Anyone can call `trigger_liquidation_auction(owner, vault_id)` when vault exceeds liquidation threshold.
2. Auction contract creates an auction tied to that vault.
3. Bidders approve token allowance to auction contract, then call `place_bid`.
4. After end time, call `finalize_auction` on auction contract.
5. Vault settlement: `settle_liquidation_auction(owner, vault_id)`.

### 4) Admin flow

Privileged protocol actions are gated by each contract's `governance` role. Before hand-off this is
the deployer; after wiring (deployment steps 7 & 9) it is the **governance contract**, and you drive
them through governance's forwarders rather than calling the protocol contracts directly. See
[Governance & Treasury](#governance--treasury) for who may invoke what.

Risk params (the vault's `set_contract_params`) are applied behind a timelock:
`collateral_ratio`, `liquidation_ratio`, `interest_rate`, `liquidation_fee`, `transaction_fee`,
`borrow_cap`, per-vault and total collateral caps, `auction_duration_ms`, `max_oracle_age_ms`.

Oracle reporter access (`set_reporter`) is managed by the oracle's validator; the validator and the
max price deviation are governance-set. The active round is committed by the validator via
`commit_round`; governance can also commit an emergency override price (see below).

Default vault params:

- Collateral ratio: `150%`
- Liquidation ratio: `120%`
- Interest rate: `5% APR` (approximately `5.13% APY` under hourly compounding)
- Liquidation fee: `1%`
- Auction duration: `3_600_000` milliseconds
- Max oracle age: `3_600_000` milliseconds

## Governance & Treasury

### Authorities

`tusdt-governance` carries two roles in addition to token-holder voting:

- **Maintainer** — the top authority (typically the subnet owner; an election contract is expected
  to hold this role later). Passed in at construction. The maintainer transfers its own role
  (`set_maintainer`), seats the council (`set_council`), updates governance parameters
  (`update_params`), and drives the protocol config forwarders below.
- **Council** — a fixed committee of exactly 5 members set by the maintainer. The council performs
  operational duties: committing voting snapshots (`submit_snapshot`) and the emergency vault halt
  (`vault_pause`). Any single council member can act on these.

### Token-holder proposals

Funding and signal proposals are decided by token-weighted voting against a committed Merkle
snapshot (voting power is `sqrt(snapshot balance) × time-staked multiplier`, so flash-staking can't
inflate it). Flow: `submit_proposal` → `vote` → `finalize` → `execute`. Submission is gated on the
proposer's subnet alpha stake and a monthly submission window; passing requires both quorum (a
fraction of circulating supply) and an approval threshold. A passed **Funding** proposal calls
`treasury.release(...)` on execution; **NonFunding** proposals are signal-only.

### Steering the protocol (forwarders)

After the role hand-off, the governance contract holds the `governance` role on the vault, auction,
and oracle. It exposes thin forwarders that perform the cross-contract call; the protocol contracts
see the governance contract as their `governance` caller. Authorization is decided inside
governance:

| Forwarder | Gated by | Target |
| --- | --- | --- |
| `vault_set_contract_params`, `vault_cancel_contract_params_update` | maintainer | vault (timelocked params) |
| `vault_update_treasury`, `vault_update_platform`, `vault_unpause` | maintainer | vault |
| `vault_pause` | **council** (fast emergency halt) | vault |
| `oracle_set_validator`, `oracle_set_max_price_deviation` | maintainer | oracle |
| `oracle_commit_round` | maintainer (emergency price — drives liquidations) | oracle |
| `auction_set_admin` | maintainer | auction |

`update_governance` on the protocol contracts is intentionally **not** forwarded — the vault's role
is fixed after wiring (and it propagates role changes to the auction/oracle itself).

### Treasury

`tusdt-treasury` books protocol fees into named funds — `Emergency`, `Operation`, `Insurance`,
`Dividend`, `Buyback`, `Voting` — in both `Tusdt` and `Native` denominations. `distribute()` splits
incoming balance across the funds; `release(fund, token_kind, amount, recipient)` pays out and is
callable **only by governance** (i.e. via an executed Funding proposal). The deployer is the initial
governance until `set_governance` hands control to the governance contract.

## Useful Read Methods

- Vault: `get_vault`, `get_total_debt`, `get_contract_params`, `get_oracle_address`, `get_vaults`, `get_all_vaults`
- Oracle: `get_latest_price`, `get_current_round_summary`, `is_reporter`
- Auction: `get_auction`, `get_active_vault_auction`, `get_bid`, `get_all_auctions`, `get_active_auctions`
- Token: `balance_of`, `allowance`, `total_supply`
- Governance: `maintainer`, `council`, `is_council`, `params`, `current_epoch`, `get_snapshot`, `quorum`, `proposal_count`, `get_proposal`, `has_voted`
- Treasury: `governance`, `token`, `fund_balance_tusdt`, `fund_balance_native`

## Notes

- `tusdt-vault` owns the token and auction instances it creates.
- `tusdt-vault` reads collateral pricing from the external oracle contract.
- Borrowing mints TUSDT to borrower.
- Repayment and settlement burn TUSDT.
- Protocol fees accrue to `tusdt-treasury`; only `tusdt-governance` can release them.
- After wiring, the vault/auction/oracle are governed by `tusdt-governance`; the maintainer and
  council act through its forwarders rather than calling those contracts directly.
