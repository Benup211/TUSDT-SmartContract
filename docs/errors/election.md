# Election — Error catalog

**Contract:** `contracts/tusdt-election/lib.rs` (package `tusdt-election`, artifact `tusdt_election`)

**Enum:** `pub enum Error` — all variants fieldless, `#[derive(Debug, PartialEq, Eq)]` + `#[ink::scale_derive(Encode, Decode, TypeInfo)]`. 22 variants.

**Enum doc comment (verbatim):**

> Errors returned by the election contract.

Fieldless `Error` enum: each variant encodes as a single `u8` SCALE index. See [index.md](index.md) for how errors surface in dedot/dApp/CLI clients.

**Reserved variants** (declared, not returned by any current message): `NoEmergencyPending`.

## Variants

### `NotIncumbent`

> The caller is not the current incumbent maintainer.

**Returned by:** `advance_cadence` (helper), `ensure_incumbent` (helper)

**Client guidance:** Authorization: only the current incumbent maintainer may call.

### `WrongPhase`

> The operation is not allowed in the current election phase.

**Returned by:** `activate` (message), `cancel_cycle` (message), `cast_approval` (message), `finalize` (message), `register_candidate` (message), `schedule_election` (message), `trigger_emergency_election` (message), `test_schedule` (helper)

**Client guidance:** State error: the action is not allowed in the current phase - check the cycle phase first.

### `NotElectionTime`

> The cadence anchor has not been reached and no emergency is pending.

**Returned by:** `cast_approval` (message), `schedule_election` (message), `test_schedule` (helper)

**Client guidance:** Timing: the cadence anchor has not been reached and no emergency is pending - wait.

### `VotingClosed`

> The voting window has ended; no more votes are accepted.

**Returned by:** `cast_approval` (message)

**Client guidance:** State error: the voting window ended; no further votes are accepted.

### `VotingStillOpen`

> Voting has not yet ended; finalization is not allowed.

**Returned by:** `activate` (message), `finalize` (message)

**Client guidance:** State error: wait for voting to end before finalizing/activating.

### `BeforeActivationDay`

> The current day-of-month is before the activation day (the 15th).

**Returned by:** `activate` (message)

**Client guidance:** Timing: activation is only allowed on/after the 15th of the month; retry on the activation day.

### `CandidateNotFound`

> No candidate with the given account was found in the current cycle.

**Returned by:** `cast_approval` (message), `finalize` (message)

**Client guidance:** Check the candidate address against the current cycle's registrations; retry with a valid candidate.

### `AlreadyRegistered`

> The caller is already a registered candidate for this cycle.

**Returned by:** `register_candidate` (message)

**Client guidance:** Idempotency: already a candidate for this cycle; do not retry.

### `TermLimitReached`

> The candidate has already served the maximum number of terms (`MAX_TERMS`).

**Returned by:** `register_candidate` (message)

**Client guidance:** Eligibility: the candidate hit `MAX_TERMS`; not eligible for this cycle.

### `InvalidNetuid`

> The provided subnet netuid is invalid (e.g., zero).

**Returned by:** `register_candidate` (message)

**Client guidance:** Input error: provide a valid non-zero netuid.

### `InsufficientStake`

> The candidate's alpha stake is below the minimum candidate stake threshold.

**Returned by:** `register_candidate` (message)

**Client guidance:** Eligibility: the candidate's subnet alpha stake is below the minimum; stake more and retry.

### `StakeQueryFailed`

> The chain extension call to query stake information failed.

**Returned by:** `ensure_incumbent` (helper), `read_candidate_stake` (helper)

**Client guidance:** Chain-extension failure: retry (may be transient); report if persistent.

### `NoSnapshot`

> No electorate snapshot has been committed for the current cycle.

**Returned by:** `cast_approval` (message), `gov_latest_snapshot` (helper), `read_candidate_stake` (helper)

**Client guidance:** State error: wait for the council to commit the electorate snapshot.

### `InvalidProof`

> The Merkle proof does not match the snapshot root.

**Returned by:** `cast_approval` (message)

**Client guidance:** Input error: the Merkle proof does not match the snapshot root - recompute.

### `NoStake`

> The computed voting power is zero; the leaf has no stake.

**Returned by:** `cast_approval` (message), `ensure_incumbent` (helper), `read_candidate_stake` (helper)

**Client guidance:** Eligibility: the leaf has zero voting power.

### `AlreadyApproved`

> This (coldkey, hotkey) pair has already voted in the current cycle.

**Returned by:** `cast_approval` (message)

**Client guidance:** Idempotency: this (coldkey, hotkey) pair already voted this cycle.

### `NoWinner`

> No candidate achieved a majority in the election.

**Returned by:** `activate` (message), `finalize` (message)

**Client guidance:** State error: no majority winner - the election must be rerun.

### `NoTransition`

> No cross-subnet transition is currently active.

**Returned by:** `end_transition` (message)

**Client guidance:** State error: no cross-subnet transition is active; informational.

### `TransitionActive`

> The transition period has not yet elapsed; the netuid switch is deferred.

**Returned by:** `end_transition` (message)

**Client guidance:** Timing: the transition period has not elapsed - the netuid switch is deferred; wait.

### `NoEmergencyPending`

> No emergency election has been flagged by the incumbent.

**Returned by:** *no production code returns this variant (reserved)*

**Client guidance:** Reserved: declared but not returned by any current message. Report if observed on-chain.

### `GovernanceCallFailed`

> The cross-contract call to governance failed.

**Returned by:** `gov_elect_maintainer` (helper), `gov_election_set_netuid` (helper), `gov_latest_snapshot` (helper)

**Client guidance:** Cross-contract failure to governance: retry; report if persistent.

### `ArithmeticError`

> Arithmetic overflow or underflow occurred.

**Returned by:** `activate` (message), `cast_approval` (message), `finalize` (message), `advance_cadence` (helper), `open_registration` (helper)

**Client guidance:** Unexpected numeric error: report as a contract bug.

## Decoding note (election -> governance calls)

> Governance's `Error` is a fieldless enum, so its `Result<(), Error>` decodes layout-compatibly as `Result<(), u8>`; any callee failure is mapped to `Error::GovernanceCallFailed`. *(source comment, `gov_elect_maintainer`)*
