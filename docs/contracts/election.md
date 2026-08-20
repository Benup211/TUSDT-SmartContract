# Election — electing the maintainer

`tusdt-election` is the **on-chain leader election** for the protocol's top authority — the maintainer account that sits atop governance. Every ~2 years the electorate votes a subnet owner in; the winner is installed into governance as the new maintainer. Think of it as an on-chain presidential election fused with a delegated-executive model: the people (snapshot electorate) pick, the contract installs. It is instantiated **by** `tusdt-governance` at construction and calls back into governance with raw 4-byte selectors to avoid a crate dependency cycle.

## How it works

### The cycle

Lifecycle: `Idle → Registration → Voting → Elected → Idle`. A term is `TERM_LENGTH_MS` = 730 days (2 years); the next election may open at $next\_election\_ts = genesis\_election\_ts + term\_index \times 730d$ (`advance_cadence`), recomputed from the genesis anchor so no millisecond drift accumulates. The initial maintainer counts as term 1 — the first election falls one full term after deployment.

### Snapshot from governance

`schedule_election` (permissionless, once the cadence anchor is reached or an emergency is pending) pulls the electorate from governance's latest committed snapshot via `election_snapshot` (selector `0xE1EC7002`) — the same Merkle root, leaf encoding, and voting-power formula as governance proposals, so one off-chain snapshot generator serves both.

### Candidates

`register_candidate(netuid, hotkey)` is permissionless during `Registration`, gated on-chain:

- `netuid` must be non-zero (`InvalidNetuid`); the account must have served fewer than `MAX_TERMS` (2) terms (`TermLimitReached`).
- The coldkey must hold at least `MIN_CANDIDATE_STAKE` = $10^{13}$ rao (10 000 TAO) of alpha in `netuid`, proven live via the chain extension (`read_candidate_stake`) — the subnet-owner bar. No approval step: a registered candidate is immediately votable.

### Voting: one leaf, one approval

`cast_approval(candidate, hotkey, balance, multiplier_bps, proof)` is the single-vote rule: each `(coldkey, hotkey)` leaf may approve **exactly one candidate** (`AlreadyApproved`). The first vote of a cycle — only on UTC days 5–9 — opens voting and pins `voting_ends_at` to the 10th at 00:00 UTC, so the window is fixed to the calendar regardless of when voting actually starts. Each vote proves its leaf against the cycle's snapshot root (`verify_merkle_proof`), contributes weight

$$\text{weight} = \left\lfloor \sqrt{balance} \times \frac{multiplier\_bps}{10\,000} \right\rfloor$$

to the candidate's approval total, and the contract keeps a **running leader** (`leading_candidate`) so `finalize` never scans candidates.

### Finalize: quorum + strict majority

`finalize` (permissionless, after `voting_ends_at`) declares a winner only if **both** hold:

- **Turnout quorum** — raw voted balance: $total\_voted\_balance \ge \lfloor circulating\_supply \times 2\,000 / 10\,000 \rfloor$ (20%).
- **Strict majority** — $best \times 10\,000 > total\_voting\_power \times 5\,000$: the leader's approval weight must exceed 50% of the participating voting power. Since every leaf votes once, at most one candidate can clear this bar.

No winner → the incumbent stays and the cadence advances (`ElectionFinalized` with `winner: None`).

### Activation: installing the maintainer

`activate` (permissionless, on/after UTC day 15 — `ACTIVATION_DAY`) installs the winner into governance via the raw cross-contract call `elect_maintainer` (selector `0xE1EC7000`), bumps the winner's `terms_served`, and advances the cadence. If the winner governs a **different subnet**, a 182-day transition (`TRANSITION_MS`) opens: the previous subnet stays authoritative for snapshots and the netuid switch is deferred to `end_transition`, which pushes it into governance via `election_set_netuid` (selector `0xE1EC7001`).

### Incumbent powers

The incumbent may `trigger_emergency_election` (lets the next `schedule_election` bypass the cadence anchor, e.g. after subnet deregistration) and `cancel_cycle` — but **only during `Registration`**, so an outgoing incumbent can never veto a decided winner.

## User flow

1. **Schedule** — anyone calls `schedule_election()` once `next_election_ts` has passed (or an emergency is pending).
2. **Stand** — a subnet owner calls `register_candidate(netuid, hotkey)` with ≥ 10 000 TAO staked.
3. **Vote** — holders call `cast_approval(...)` between the 5th and the 10th (UTC); one approval per leaf.
4. **Finalize** — anyone calls `finalize()` after the 10th; quorum + strict majority decide.
5. **Activate** — anyone calls `activate()` on/after the 15th; the winner becomes governance's maintainer (and seats their own council there).
6. **Migrate** — if the subnet changed, anyone calls `end_transition()` after 182 days to flip the governing netuid.

## Key parameters

| Parameter | Meaning | Default | Scale |
| --- | --- | --- | --- |
| `TERM_LENGTH_MS` | Term between elections | 730 days | ms |
| `MAX_TERMS` | Max terms per account | 2 | count |
| `VOTE_OPEN_DAY` / `VOTE_CLOSE_DAY` | Voting window (UTC) | 5 / 10 | day of month |
| `ACTIVATION_DAY` | Earliest winner installation | 15 | day of month |
| `APPROVAL_THRESHOLD_BPS` | Strict majority bar | 5 000 (> 50%) | bps, /10 000 |
| `QUORUM_BPS` | Turnout quorum of circulating supply | 2 000 (20%) | bps, /10 000 |
| `MIN_CANDIDATE_STAKE` | Candidate subnet-owner bar | 10 000 000 000 000 rao (10 000 TAO) | rao |
| `TRANSITION_MS` | Cross-subnet migration | 182 days | ms |
| Weight formula | Per-leaf voting power | $\lfloor\sqrt{balance}\cdot mult/10\,000\rfloor$ | rao |

## Talks to

- **`tusdt-governance`** — installs the winner (`elect_maintainer`, `0xE1EC7000`), switches netuids (`election_set_netuid`, `0xE1EC7001`), reads the electorate (`election_snapshot`, `0xE1EC7002`).
- **`tusdt-voting`** (shared crate) — `voting_power`, `integer_sqrt`, `leaf_hash`, `verify_merkle_proof`, `day_of_month`.
- **Bittensor chain extension** — `get_stake_info_for_hotkey_coldkey_netuid` for the candidate stake bar.

## Errors

Full catalog: [error reference](../errors/election.md). Notable ones: `NotIncumbent`, `WrongPhase`, `NotElectionTime`, `VotingClosed`, `VotingStillOpen`, `BeforeActivationDay`, `CandidateNotFound`, `AlreadyRegistered`, `TermLimitReached`, `InvalidNetuid`, `InsufficientStake`, `NoSnapshot`, `InvalidProof`, `NoStake`, `AlreadyApproved`, `NoWinner`, `NoTransition`, `TransitionActive`, `GovernanceCallFailed`, `ArithmeticError`. One variant is reserved and never returned: `NoEmergencyPending`.
