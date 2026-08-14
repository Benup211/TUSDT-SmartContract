# Governance — the protocol steerer

`tusdt-governance` is the top-level authority of the TUSDT protocol. It combines three layers in one contract: an **elected maintainer** (the executive), a **5-member council** (the operations committee), and **token-holder proposals** (the legislature) that can spend the treasury. Once wired, it also steers the vault, auction, oracle, and lending pool through thin forwarding messages — those contracts treat governance as their `governance` caller.

The closest DeFi cousins: **Compound Governor Alpha** (proposal → vote → queue → execute lifecycle with quorum + threshold), **MakerDAO governance** (elected executives with a delay before activation), and a **Timelock/Guardian** in the form of the maintainer + council.

## How it works

### Two authorities, one steward

| Role | Who | Powers | Replaced by |
| --- | --- | --- | --- |
| **Maintainer** | Set at construction (`new`), typically the subnet owner | All parameter updates (`update_params`), council seating (`set_council`), every maintainer-gated forwarder below | **Only** the election contract, via `elect_maintainer` (selector `0xE1EC7000`) |
| **Council** | Exactly 5 distinct accounts, seated by the maintainer (`set_council`) | Operational duties: `submit_snapshot`, `vault_pause`, `submit_proposal` — any single member can act, no consensus needed | Maintainer (`set_council` re-seats at will) |

The maintainer is an *account*, not a contract — it is the elected subnet owner. Governance **instantiates the election contract** in its own constructor (`new`, from `election_code_hash`) and authorizes it to install future maintainers and switch the governing subnet (`election_set_netuid`).

### Token-holder proposals

Proposals move through a fixed lifecycle: `Active → Passed / Rejected → Executed`.

- **`ProposalKind::Funding`** carries `{fund, token_kind, amount, recipient}`. On execution it calls `treasury.release(fund, token_kind, amount, recipient)` (`execute`).
- **`ProposalKind::NonFunding`** is signal-only: a passed NonFunding proposal just records the outcome (`execute` skips the treasury call).

Submission (`submit_proposal`) is **council-gated** and additionally restricted to a **monthly window** — by default the 20th–27th of each month, UTC (`submission_open_day`/`submission_close_day`, computed with `day_of_month`). Each proposal carries an IPFS `cid` (≤ 96 bytes, `MAX_CID_LEN`) pointing at the off-chain proposal document, and binds to the snapshot epoch committed at submission time.

### Voting power: anti-flash-stake quadratic weighting

The electorate is an **off-chain Merkle snapshot** committed on-chain by the council (`submit_snapshot`). Each leaf is

$$leaf = blake2\_256(SCALE(coldkey,\ hotkey,\ balance,\ multiplier\_bps))$$

(`leaf_hash` in `tusdt-voting`). `balance` is the alpha balance frozen at the snapshot block, so buying stake after the snapshot changes nothing. A vote proves its leaf against the snapshot root with a Merkle proof (`verify_merkle_proof`), and each `(coldkey, hotkey)` pair votes once per proposal (`AlreadyVoted` otherwise).

Voting power is quadratic in balance and scaled by a time-staked multiplier (`voting_power` in `tusdt-voting`):

$$\text{weight} = \left\lfloor \sqrt{balance} \cdot \frac{multiplier\_bps}{10\,000} \right\rfloor$$

The square root dampens whales (10,000× the stake is only 100× the power); `multiplier_bps` rewards longer-held stake per leaf (e.g. 1.0× = 10 000, 0.5× = 5 000). The integer square root is Newton's method (`integer_sqrt`) — no floats on-chain.

### Quorum and approval — two different measures

`finalize` applies two gates (both must hold, plus `total > 0`):

- **Quorum** is measured in **raw snapshot balance**, not voting power: the sum of the `balance` of every leaf that voted must reach `quorum(epoch) = ⌊circulating_supply × quorum_bps / 10 000⌋` — by default 20% of the snapshot's alpha circulating supply.
- **Approval** is measured in **voting power**: $\lfloor yes \times 10\,000 / (yes + no) \rfloor \ge approval\_bps$ — by default 5 001 bps (50.01%).

Both `finalize` and `execute` are **permissionless** — anyone may close voting once `voting_ends_at` has passed and push a passed proposal through, so the lifecycle can never stall.

### Forwarders: steering the protocol

After the role hand-off (below), governance holds the `governance` role on the vault, auction, oracle, and lending pool. Each forwarder below is a thin message that performs the cross-contract call; the target contract sees governance as its authorized `governance` caller. Authorization is decided inside governance (`ensure_maintainer` vs `ensure_council` in `external_calls.rs`):

| Message | Gate | Purpose |
| --- | --- | --- |
| `vault_set_contract_params(netuid, params)` / `vault_cancel_contract_params_update` | maintainer | Timelocked per-netuid vault params (collateral/liquidation ratios, fees) |
| `vault_set_global_params(config)` / `vault_cancel_global_params_update` | maintainer | Timelocked global vault params (transaction fee, auction duration, max oracle age) |
| `vault_set_approved_netuid(netuid, approved)` | maintainer | Whitelist subnets as alpha collateral |
| `vault_update_treasury` / `vault_update_platform` | maintainer | Vault's fee recipient / pause operator |
| `vault_set_token_controller(new)` | maintainer | Hand the ERC20 controller to a new vault on upgrade |
| `vault_update_auction_address` / `vault_update_oracle_address` | maintainer | Repoint vault children after upgrades |
| `vault_set_hotkey(new_hotkey, netuids)` | maintainer | Migrate the vault's staking hotkey (`move_stake`); blocked during liquidations |
| `vault_claim_excess_alpha(netuid)` | maintainer | Claim excess staked alpha → TAO to treasury |
| `vault_transfer_native_to_treasury()` | maintainer | Sweep the vault's native TAO balance to the treasury |
| `vault_unpause` | maintainer | Deliberate recovery from a pause |
| `vault_get_hotkey` | none (read) | Read the vault's staking hotkey |
| `vault_pause` | **council** | Fast emergency halt — the one operational call, any single member |
| `oracle_set_validator(Option)` | maintainer | Seat/clear the oracle round committer |
| `oracle_set_max_price_deviation(ratio)` | maintainer | Widen/tighten the price deviation band |
| `oracle_set_netuid` / `oracle_set_min_submitter_stake` | maintainer | Oracle reporter subnet + stake bar |
| `oracle_commit_round(price)` | maintainer | **Emergency price override** — bypasses quorum/deviation, drives liquidations |
| `auction_set_admin(Option)` | maintainer | Who may bid on expired no-bid auctions |
| `pool_set_approved_netuid` | maintainer | Lending-pool collateral subnets |
| `pool_set_alpha_params` / `pool_set_market_params` / `pool_set_global_params` (+ cancels) | maintainer | Timelocked lending-pool parameters |
| `pool_update_platform` / `pool_update_treasury` / `pool_update_oracle_address` / `pool_update_ltoken_address` | maintainer | Pool wiring and fee recipient |
| `pool_update_pool_hotkey` / `pool_update_maintainer` | maintainer | Pool hotkey migration / maintainer hand-off |
| `pool_claim_surplus_tusdt(amount)` / `pool_transfer_native_to_treasury` / `pool_unpause` | maintainer | Sweep pool surplus (reserve + performance fees) to treasury; unpause |
| `update_vault_address` / `update_pool_address` / `update_auction_address` / `update_oracle_address` / `update_treasury_address` | maintainer | Repoint governance's own stored refs after upgrades |

The stored `pool` ref starts at the zero address (`from_addresses`); the maintainer must call `update_pool_address` after the lending pool is deployed.

### Role hand-off

`update_governance` on the protocol contracts is **deliberately not forwarded**. At wiring time the deployer hands roles directly: `treasury.set_governance(governance)` and `vault.update_governance(governance)` — the vault then **propagates** the new role to its child auction and oracle (`sync_child_governance`). From then on the vault's role is fixed; only the vault (or its governance-gated setters) can move it again, e.g. `vault_set_token_controller` on a vault upgrade.

## User flow

### Submit, vote, finalize, execute

1. **Snapshot** — a council member calls `submit_snapshot(root, circulating_supply, snapshot_block)` to commit the off-chain electorate Merkle root. Each call advances the epoch by one; check `current_epoch()`.
2. **Propose** — within the monthly window (UTC day 20–27 by default), a council member calls `submit_proposal(cid, kind)` with a valid IPFS CID and either a `NonFunding` or `Funding { fund, token_kind, amount, recipient }` payload. `voting_ends_at = created_at + voting_period_ms` (7 days).
3. **Vote** — a token holder calls `vote(proposal_id, hotkey, support, balance, multiplier_bps, proof)`; the caller is the coldkey. The leaf must match the proposal's snapshot root, the pair must not have voted, and weight > 0.
4. **Finalize** — after `voting_ends_at`, anyone calls `finalize(proposal_id)`. Quorum + approval decide `Passed` vs `Rejected`.
5. **Execute** — for a `Passed` proposal anyone calls `execute(proposal_id)`. Funding proposals trigger `treasury.release(...)`; NonFunding proposals just record `Executed`.

### Emergency halt (council)

Any single council member calls `vault_pause()` to halt the vault immediately; recovery is maintainer-only via `vault_unpause()`.

## Key parameters

| Parameter | Meaning | Default | Scale |
| --- | --- | --- | --- |
| `voting_period_ms` | Time from submission to close | 604 800 000 ms (7 days) | ms |
| `quorum_bps` | Quorum as fraction of snapshot circulating supply | 2 000 (20%) | bps, /10 000 |
| `approval_bps` | Min yes-share of voting power | 5 001 (50.01%) | bps, /10 000 |
| `min_proposer_stake` | Declared but **unused** (council-only submission model; kept for SCALE compatibility) | 1 000 000 000 000 | rao |
| `submission_open_day` / `submission_close_day` | Monthly UTC window for `submit_proposal` | 20 / 27 | day of month |
| `COUNCIL_SIZE` | Council membership (`set_council`) | 5 | — |
| `MAX_CID_LEN` | Max proposal CID length | 96 | bytes |
| `DEFAULT_NETUID` | Governing subnet at construction | 113 | netuid |
| Multiplier / balance | Voting power leaf inputs | $\lfloor\sqrt{balance}\cdot mult/10\,000\rfloor$ | balance in rao |

## Talks to

- **`tusdt-treasury`** — `release(...)` on executed Funding proposals.
- **`tusdt-vault-alpha`** — governance-role forwarders (params, hotkey, pause, fee sweeps).
- **`tusdt-auction`**, **`tusdt-oracle`**, **`tusdt-lending-pool`** — governance-role forwarders.
- **`tusdt-election`** — instantiated by governance at construction; the only caller allowed on `elect_maintainer` / `election_set_netuid`, and the consumer of `election_snapshot`.
- **`tusdt-voting`** (shared crate) — `voting_power`, `integer_sqrt`, `leaf_hash`, `verify_merkle_proof`, `day_of_month`.

## Errors

Full catalog: [error reference](../errors/governance.md). Notable ones: `NotMaintainer`, `NotElection`, `NotCouncil`, `InvalidCouncil`, `ProposalNotFound`, `ProposalNotActive`, `VotingClosed`, `VotingStillOpen`, `AlreadyVoted`, `AlreadyExecuted`, `NoStake`, `OutsideSubmissionWindow`, `NoSnapshot`, `InvalidProof`, `InvalidCid`, `InvalidAmount`, `InvalidParams`, `NotPassed`, `TreasuryCallFailed`, `VaultCallFailed`, `AuctionCallFailed`, `PoolCallFailed`, `OracleCallFailed`, `VaultTokenCallFailed`, `ArithmeticError`. Two variants are reserved and never returned: `InsufficientStake`, `StakeQueryFailed`.
