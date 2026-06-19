#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use self::election::{
    Candidacy, CandidateRegistered, ElectionFinalized, ElectionScheduled, ElectionSnapshot,
    EmergencyElectionTriggered, Error, MaintainerActivated, MaintainerElect, Phase, Transition,
    TransitionEnded, TransitionStarted, TusdtElection, TusdtElectionRef,
};

#[ink::contract(env = tusdt_env::CustomEnvironment)]
mod election {
    use ink::env::call::{build_call, ExecutionInput, Selector};
    use ink::prelude::vec::Vec;
    use ink::storage::Mapping;

    // Raw cross-contract call selectors into `tusdt-governance`. The election deliberately does NOT
    // depend on the governance crate (that would be a dependency cycle, since governance now
    // instantiates the election). Instead it calls governance's election-gated messages by their
    // explicit selectors — these MUST stay in sync with the `#[ink(message, selector = ...)]`
    // attributes on `TusdtGovernance::elect_maintainer` / `election_set_netuid` / `election_snapshot`.
    const SELECTOR_ELECT_MAINTAINER: [u8; 4] = [0xE1, 0xEC, 0x70, 0x00];
    const SELECTOR_ELECTION_SET_NETUID: [u8; 4] = [0xE1, 0xEC, 0x70, 0x01];
    const SELECTOR_ELECTION_SNAPSHOT: [u8; 4] = [0xE1, 0xEC, 0x70, 0x02];

    // Snapshot/voting helpers (Merkle hashing/proofs, quadratic time-staked weighting, day-of-month)
    // live in the shared `tusdt-voting` crate, byte-for-byte identical to governance's, so one
    // off-chain snapshot generator serves both. Re-exported `pub(crate)` for unqualified use here
    // and reachability from the tests.
    pub(crate) use tusdt_voting::*;

    pub(crate) const MS_PER_DAY: u64 = tusdt_voting::MS_PER_DAY;
    /// One term ≈ 2 years. The exact "5th of the month" alignment is preserved by recomputing the
    /// next-election anchor from `genesis_election_ts + term_index * TERM_LENGTH_MS` each cycle
    /// (no ms drift accumulates); the day-of-month vote/activation gates re-align the calendar.
    pub(crate) const TERM_LENGTH_MS: u64 = 730 * MS_PER_DAY;
    /// Day-of-month (UTC) on which voting opens (the 5th).
    pub(crate) const VOTE_OPEN_DAY: u8 = 5;
    /// Day-of-month (UTC) on which voting closes (the 10th); the window is the 5th up to the 10th,
    /// fixed to the calendar regardless of when the first vote lands.
    pub(crate) const VOTE_CLOSE_DAY: u8 = 10;
    /// Earliest day-of-month (UTC) on which a winner may be activated (the 15th).
    pub(crate) const ACTIVATION_DAY: u8 = 15;
    /// Migration window after a different-subnet winner activates (≈ 6 months). During it the
    /// previous governing subnet's snapshot stays authoritative and the netuid switch is deferred.
    pub(crate) const TRANSITION_MS: u64 = 182 * MS_PER_DAY;
    /// A maintainer (account) may serve at most this many terms.
    pub(crate) const MAX_TERMS: u32 = 2;
    /// Approval threshold in basis points; a winner needs strictly more than 50%.
    pub(crate) const APPROVAL_THRESHOLD_BPS: u32 = 5_000;
    /// Turnout quorum in basis points of the snapshot's alpha circulating supply (2_000 = 20%),
    /// mirroring governance proposals. The raw balance that voted must reach this for any winner.
    pub(crate) const QUORUM_BPS: u32 = 2_000;
    /// Default alpha a candidate's coldkey must hold in the subnet it registers for, to be treated
    /// as that subnet's owner (the registration "owner" bar). Used directly by `register_candidate`.
    pub(crate) const MIN_CANDIDATE_STAKE: u128 = 1_000_000_000_000;

    /// Lifecycle stage of the current election cycle.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub enum Phase {
        /// Between cycles; the incumbent governs.
        Idle,
        /// Cycle is scheduled (electorate snapshot fetched from governance); candidates register.
        Registration,
        /// Voting is open; opened lazily by the first vote and runs to [`VOTE_CLOSE_DAY`] (the 10th).
        Voting,
        /// A winner is recorded; awaiting `activate` (on/after the 15th).
        Elected,
    }

    /// A registered candidacy for the current cycle. Eligibility is gated on-chain by the
    /// registrant's alpha stake in `netuid` (see [`TusdtElection::register_candidate`]) — there is
    /// no separate approval step; a registered candidate is immediately votable.
    #[derive(Debug, Clone)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct Candidacy {
        /// The subnet the candidate owns and would govern from if elected.
        pub netuid: u16,
        pub registered_at: Timestamp,
    }

    /// A committed election snapshot, fetched from governance at `schedule_election`. Leaves are
    /// `blake2_256(SCALE(coldkey, hotkey, balance, multiplier_bps))`, identical to governance's
    /// snapshot encoding. `netuid` records which subnet's holders this electorate represents (the
    /// previous subnet while a migration is deferred).
    #[derive(Debug, Clone)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct ElectionSnapshot {
        pub root: MerkleHash,
        pub circulating_supply: u128,
        pub netuid: u16,
        pub snapshot_block: u32,
    }

    /// The winner recorded at `finalize`, installed at `activate`.
    #[derive(Debug, Clone)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct MaintainerElect {
        pub who: AccountId,
        pub netuid: u16,
        pub approval_weight: u128,
        pub decided_at: Timestamp,
    }

    /// An in-flight 6-month migration after a different-subnet winner activates. Until `ends_at`,
    /// governance's snapshot (the previous subnet) stays the electorate and the netuid switch is
    /// deferred to [`TusdtElection::end_transition`].
    #[derive(Debug, Clone)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct Transition {
        pub from_netuid: u16,
        pub to_netuid: u16,
        pub ends_at: Timestamp,
    }

    /// Election storage.
    #[ink(storage)]
    pub struct TusdtElection {
        /// Governance contract address. This contract is authorized as governance's `election` (so
        /// it may call `elect_maintainer` / `election_set_netuid` and read `election_snapshot` via
        /// raw cross-contract calls).
        governance: AccountId,
        /// Genesis cadence anchor (ms): the deployment block timestamp.
        genesis_election_ts: Timestamp,
        /// Elapsed terms, used as the cadence multiplier (`next = genesis + term_index * term`, no
        /// ms drift). Starts at 1: the initial incumbent is treated as just elected at genesis, so
        /// the first election falls one full term later.
        term_index: u32,
        /// Earliest timestamp the next election may be scheduled.
        next_election_ts: Timestamp,
        /// The current effective maintainer (the elected subnet owner).
        incumbent: AccountId,
        /// The current governing subnet (mirrors governance's netuid; updated at `end_transition`).
        active_netuid: u16,
        /// Completed terms per account; an account at [`MAX_TERMS`] cannot register.
        terms_served: Mapping<AccountId, u32>,
        phase: Phase,
        /// Monotonic cycle id; namespaces all per-cycle maps so a new cycle starts clean.
        cycle_id: u64,
        candidates: Mapping<(u64, AccountId), Candidacy>,
        candidate_list: Vec<AccountId>,
        /// `(cycle_id, candidate) -> accumulated approval voting power`.
        approvals: Mapping<(u64, AccountId), u128>,
        /// `(cycle_id, coldkey, hotkey) -> ()`; records that a leaf has cast its single vote. Each
        /// leaf may approve exactly one candidate, so this both blocks any second vote and counts
        /// the leaf once toward `total_voting_power`.
        participated: Mapping<(u64, AccountId, AccountId), ()>,
        /// Denominator for the >50% test: summed voting power of unique participating leaves.
        total_voting_power: u128,
        /// Summed raw (snapshot-frozen) alpha balance of unique participating leaves; measured
        /// against the [`QUORUM_BPS`] turnout quorum at `finalize`.
        total_voted_balance: u128,
        /// Running leader, maintained on every vote so `finalize` need not scan candidates. Since
        /// each leaf votes once, candidate totals sum to `total_voting_power`, so at most one
        /// candidate can exceed 50% — the running max identifies the winner exactly.
        leading_candidate: Option<AccountId>,
        leading_weight: u128,
        election_snapshot: Option<ElectionSnapshot>,
        voting_opens_at: Timestamp,
        voting_ends_at: Timestamp,
        /// `None` ⇒ no candidate cleared 50% ⇒ the incumbent stays.
        maintainer_elect: Option<MaintainerElect>,
        transition: Option<Transition>,
        /// Set by the incumbent; lets the next `schedule_election` bypass the cadence anchor.
        emergency_pending: bool,
    }

    // ------------------------------------------------------------------------------------------- //
    // Events                                                                                       //
    // ------------------------------------------------------------------------------------------- //

    #[ink(event)]
    pub struct ElectionScheduled {
        #[ink(topic)]
        cycle_id: u64,
        emergency: bool,
        scheduled_at: Timestamp,
    }

    #[ink(event)]
    pub struct CandidateRegistered {
        #[ink(topic)]
        cycle_id: u64,
        #[ink(topic)]
        candidate: AccountId,
        netuid: u16,
    }

    #[ink(event)]
    pub struct ApprovalCast {
        #[ink(topic)]
        cycle_id: u64,
        #[ink(topic)]
        candidate: AccountId,
        #[ink(topic)]
        coldkey: AccountId,
        weight: u128,
    }

    #[ink(event)]
    pub struct ElectionFinalized {
        #[ink(topic)]
        cycle_id: u64,
        winner: Option<AccountId>,
        netuid: u16,
        approval_weight: u128,
        total_voting_power: u128,
    }

    #[ink(event)]
    pub struct MaintainerActivated {
        #[ink(topic)]
        cycle_id: u64,
        #[ink(topic)]
        new_maintainer: AccountId,
        active_netuid: u16,
    }

    #[ink(event)]
    pub struct TransitionStarted {
        from_netuid: u16,
        to_netuid: u16,
        ends_at: Timestamp,
    }

    #[ink(event)]
    pub struct TransitionEnded {
        netuid: u16,
    }

    #[ink(event)]
    pub struct EmergencyElectionTriggered {
        #[ink(topic)]
        triggered_by: AccountId,
    }

    /// Errors returned by the election contract.
    #[derive(Debug, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    pub enum Error {
        NotIncumbent,
        WrongPhase,
        NotElectionTime,
        VotingClosed,
        VotingStillOpen,
        BeforeActivationDay,
        CandidateNotFound,
        AlreadyRegistered,
        TermLimitReached,
        InvalidNetuid,
        InsufficientStake,
        StakeQueryFailed,
        NoSnapshot,
        InvalidProof,
        NoStake,
        AlreadyApproved,
        NoWinner,
        NoTransition,
        TransitionActive,
        NoEmergencyPending,
        GovernanceCallFailed,
        ArithmeticError,
    }

    pub type Result<T> = core::result::Result<T, Error>;

    impl TusdtElection {
        /// Initializes the election contract.
        ///
        /// `governance_address` is the governance contract this election drives (governance
        /// instantiates this contract and authorizes it as its `election`, and the election reads
        /// governance's snapshot at schedule). `initial_incumbent` is the seated maintainer at
        /// deployment and runs the election machinery (emergency, cancel) until the next winner
        /// activates. It is treated as having just been elected at genesis — counted as one served
        /// term, with the first election one full [`TERM_LENGTH_MS`] later — so it cannot be
        /// challenged immediately. `initial_netuid` is the governing subnet at deployment. The
        /// candidate "owner" stake bar is the [`MIN_CANDIDATE_STAKE`] constant.
        #[ink(constructor)]
        pub fn new(
            governance_address: AccountId,
            initial_incumbent: AccountId,
            initial_netuid: u16,
        ) -> Self {
            let now = Self::env().block_timestamp();
            let mut terms_served = Mapping::default();
            terms_served.insert(initial_incumbent, &1u32);
            Self {
                governance: governance_address,
                genesis_election_ts: now,
                // The initial incumbent counts as elected at genesis (term 1); the first election
                // is one full term out.
                term_index: 1,
                next_election_ts: now.saturating_add(TERM_LENGTH_MS),
                incumbent: initial_incumbent,
                active_netuid: initial_netuid,
                terms_served,
                phase: Phase::Idle,
                cycle_id: 0,
                candidates: Mapping::default(),
                candidate_list: Vec::new(),
                approvals: Mapping::default(),
                participated: Mapping::default(),
                total_voting_power: 0,
                total_voted_balance: 0,
                leading_candidate: None,
                leading_weight: 0,
                election_snapshot: None,
                voting_opens_at: 0,
                voting_ends_at: 0,
                maintainer_elect: None,
                transition: None,
                emergency_pending: false,
            }
        }

        // --------------------------------------------------------------------------------------- //
        // Views                                                                                    //
        // --------------------------------------------------------------------------------------- //

        #[ink(message)]
        pub fn governance(&self) -> AccountId {
            self.governance
        }

        #[ink(message)]
        pub fn min_candidate_stake(&self) -> u128 {
            MIN_CANDIDATE_STAKE
        }

        #[ink(message)]
        pub fn incumbent(&self) -> AccountId {
            self.incumbent
        }

        #[ink(message)]
        pub fn active_netuid(&self) -> u16 {
            self.active_netuid
        }

        #[ink(message)]
        pub fn phase(&self) -> Phase {
            self.phase
        }

        #[ink(message)]
        pub fn cycle_id(&self) -> u64 {
            self.cycle_id
        }

        #[ink(message)]
        pub fn next_election_ts(&self) -> Timestamp {
            self.next_election_ts
        }

        #[ink(message)]
        pub fn terms_served(&self, who: AccountId) -> u32 {
            self.terms_served.get(who).unwrap_or(0)
        }

        #[ink(message)]
        pub fn get_candidate(&self, candidate: AccountId) -> Option<Candidacy> {
            self.candidates.get((self.cycle_id, candidate))
        }

        #[ink(message)]
        pub fn candidate_list(&self) -> Vec<AccountId> {
            self.candidate_list.clone()
        }

        #[ink(message)]
        pub fn approval_weight(&self, candidate: AccountId) -> u128 {
            self.approvals.get((self.cycle_id, candidate)).unwrap_or(0)
        }

        #[ink(message)]
        pub fn total_participating_power(&self) -> u128 {
            self.total_voting_power
        }

        /// Raw (snapshot-frozen) alpha balance that has voted; measured against [`quorum`].
        #[ink(message)]
        pub fn total_voted_balance(&self) -> u128 {
            self.total_voted_balance
        }

        /// The turnout quorum threshold for the current cycle: `circulating_supply * QUORUM_BPS`.
        /// Returns 0 when no electorate snapshot is recorded.
        #[ink(message)]
        pub fn quorum(&self) -> u128 {
            self.election_snapshot
                .as_ref()
                .and_then(|s| {
                    s.circulating_supply
                        .saturating_mul(u128::from(QUORUM_BPS))
                        .checked_div(u128::from(BPS_DENOMINATOR))
                })
                .unwrap_or(0)
        }

        /// The running leader and its accumulated approval weight (the winner-to-be once the cycle
        /// closes, subject to the quorum and the >50% bar).
        #[ink(message)]
        pub fn leading_candidate(&self) -> Option<(AccountId, u128)> {
            self.leading_candidate.map(|c| (c, self.leading_weight))
        }

        #[ink(message)]
        pub fn maintainer_elect(&self) -> Option<MaintainerElect> {
            self.maintainer_elect.clone()
        }

        #[ink(message)]
        pub fn transition(&self) -> Option<Transition> {
            self.transition.clone()
        }

        #[ink(message)]
        pub fn is_in_transition(&self) -> bool {
            self.transition.is_some()
        }

        #[ink(message)]
        pub fn voting_window(&self) -> (Timestamp, Timestamp) {
            (self.voting_opens_at, self.voting_ends_at)
        }

        // --------------------------------------------------------------------------------------- //
        // Election lifecycle                                                                       //
        // --------------------------------------------------------------------------------------- //

        /// Opens a new election cycle (Idle → Registration). Permissionless. Allowed once the
        /// cadence anchor is reached (`now >= next_election_ts`), or immediately when an emergency
        /// has been flagged by the incumbent. Fetches the electorate snapshot from governance's
        /// latest committed snapshot (so it reflects the current — or, mid-migration, still-deferred
        /// previous — governing subnet) and resets all per-cycle state under a fresh `cycle_id`.
        #[ink(message)]
        pub fn schedule_election(&mut self) -> Result<()> {
            if self.phase != Phase::Idle {
                return Err(Error::WrongPhase);
            }
            let now = self.env().block_timestamp();
            let emergency = self.emergency_pending;
            if !emergency && now < self.next_election_ts {
                return Err(Error::NotElectionTime);
            }

            // The electorate comes from governance — not committed locally. Fails if governance has
            // no snapshot yet.
            let (root, circulating_supply, netuid, snapshot_block) = self.gov_latest_snapshot()?;
            let snapshot = ElectionSnapshot {
                root,
                circulating_supply,
                netuid,
                snapshot_block,
            };
            self.open_registration(snapshot, now, emergency)
        }

        /// Registers the caller (coldkey) as a candidate governing `netuid`. Permissionless, but the
        /// coldkey must hold at least `min_candidate_stake` alpha in `netuid` (proven on-chain via
        /// the stake chain extension for `(hotkey, caller, netuid)`) so only a plausible subnet
        /// owner can stand. Rejected if the caller has already served [`MAX_TERMS`]. No approval
        /// step — a registered candidate is immediately votable.
        #[ink(message)]
        pub fn register_candidate(&mut self, netuid: u16, hotkey: AccountId) -> Result<()> {
            if self.phase != Phase::Registration {
                return Err(Error::WrongPhase);
            }
            if netuid == 0 {
                return Err(Error::InvalidNetuid);
            }
            let candidate = self.env().caller();
            if self.terms_served.get(candidate).unwrap_or(0) >= MAX_TERMS {
                return Err(Error::TermLimitReached);
            }
            if self.candidates.contains((self.cycle_id, candidate)) {
                return Err(Error::AlreadyRegistered);
            }

            // Owner gate: enough alpha staked in the subnet being claimed.
            let stake = self.read_candidate_stake(hotkey, candidate, netuid)?;
            if stake < MIN_CANDIDATE_STAKE {
                return Err(Error::InsufficientStake);
            }

            self.candidates.insert(
                (self.cycle_id, candidate),
                &Candidacy {
                    netuid,
                    registered_at: self.env().block_timestamp(),
                },
            );
            self.candidate_list.push(candidate);

            self.env().emit_event(CandidateRegistered {
                cycle_id: self.cycle_id,
                candidate,
                netuid,
            });
            Ok(())
        }

        /// Casts this leaf's single vote for a registered `candidate`. The caller is the coldkey;
        /// `hotkey`, `balance`, and `multiplier_bps` are the caller's leaf in the cycle's snapshot,
        /// proven by `proof`. Each `(coldkey, hotkey)` leaf may approve exactly one candidate; a
        /// second `cast_approval` from the same leaf — for any candidate — is rejected.
        ///
        /// The first vote of the cycle, cast on/after the [`VOTE_OPEN_DAY`] (the 5th), moves the cycle from
        /// `Registration` to `Voting` and starts the [`VOTING_PERIOD_MS`] (5-day) window from that vote.
        #[ink(message)]
        pub fn cast_approval(
            &mut self,
            candidate: AccountId,
            hotkey: AccountId,
            balance: u128,
            multiplier_bps: u32,
            proof: Vec<MerkleHash>,
        ) -> Result<()> {
            let now = self.env().block_timestamp();
            match self.phase {
                Phase::Registration => {
                    // The window is fixed to the calendar: votes are accepted from the 5th up to
                    // the 10th. The first such vote opens voting and pins `voting_ends_at` to the
                    // 10th 00:00 UTC of the current month, so the window is the same no matter when
                    // the first vote lands. Outside [5th, 10th) the first vote can't open voting,
                    // so a cycle scheduled too late simply rolls to next month's window.
                    let day = day_of_month(now);
                    if !(VOTE_OPEN_DAY..VOTE_CLOSE_DAY).contains(&day) {
                        return Err(Error::NotElectionTime);
                    }
                    let start_of_today = now
                        .checked_div(MS_PER_DAY)
                        .and_then(|d| d.checked_mul(MS_PER_DAY))
                        .ok_or(Error::ArithmeticError)?;
                    let days_until_close =
                        u64::from(VOTE_CLOSE_DAY.checked_sub(day).ok_or(Error::ArithmeticError)?);
                    self.voting_opens_at = now;
                    self.voting_ends_at = start_of_today
                        .checked_add(
                            days_until_close
                                .checked_mul(MS_PER_DAY)
                                .ok_or(Error::ArithmeticError)?,
                        )
                        .ok_or(Error::ArithmeticError)?;
                    self.phase = Phase::Voting;
                }
                Phase::Voting => {
                    if now >= self.voting_ends_at {
                        return Err(Error::VotingClosed);
                    }
                }
                _ => return Err(Error::WrongPhase),
            }
            if !self.candidates.contains((self.cycle_id, candidate)) {
                return Err(Error::CandidateNotFound);
            }

            let coldkey = self.env().caller();
            // Single vote: each (coldkey, hotkey) leaf may approve exactly one candidate, so a leaf
            // that has already voted (for any candidate) is rejected.
            let vote_key = (self.cycle_id, coldkey, hotkey);
            if self.participated.contains(vote_key) {
                return Err(Error::AlreadyApproved);
            }

            let snapshot = self.election_snapshot.as_ref().ok_or(Error::NoSnapshot)?;
            let leaf = leaf_hash(coldkey, hotkey, balance, multiplier_bps);
            if !verify_merkle_proof(&proof, snapshot.root, leaf) {
                return Err(Error::InvalidProof);
            }

            let weight = voting_power(balance, multiplier_bps).ok_or(Error::ArithmeticError)?;
            if weight == 0 {
                return Err(Error::NoStake);
            }

            let current = self.approvals.get((self.cycle_id, candidate)).unwrap_or(0);
            let updated = current.checked_add(weight).ok_or(Error::ArithmeticError)?;
            self.approvals.insert((self.cycle_id, candidate), &updated);

            // Maintain the running leader so `finalize` need not scan candidates. Totals only grow,
            // so a strict `>` keeps the first candidate to reach a given max as leader.
            if updated > self.leading_weight {
                self.leading_weight = updated;
                self.leading_candidate = Some(candidate);
            }

            // Record the leaf's single vote: count its power once toward the >50% denominator and
            // its raw balance once toward the turnout quorum.
            self.participated.insert(vote_key, &());
            self.total_voting_power = self
                .total_voting_power
                .checked_add(weight)
                .ok_or(Error::ArithmeticError)?;
            self.total_voted_balance = self
                .total_voted_balance
                .checked_add(balance)
                .ok_or(Error::ArithmeticError)?;

            self.env().emit_event(ApprovalCast {
                cycle_id: self.cycle_id,
                candidate,
                coldkey,
                weight,
            });
            Ok(())
        }

        /// Closes voting and decides the outcome (Voting → Elected or Idle). Permissionless; only
        /// callable after the voting window. The winner is the running leader, provided turnout
        /// reached the [`QUORUM_BPS`] quorum and its approval weight is strictly greater than 50% of
        /// the participating voting power. If no candidate clears both bars, the incumbent stays and
        /// the cadence advances.
        #[ink(message)]
        pub fn finalize(&mut self) -> Result<()> {
            if self.phase != Phase::Voting {
                return Err(Error::WrongPhase);
            }
            let now = self.env().block_timestamp();
            if now < self.voting_ends_at {
                return Err(Error::VotingStillOpen);
            }

            // The leader is tracked incrementally during voting (see `cast_approval`).
            let best = self.leading_weight;
            let winner = self.leading_candidate;

            // Turnout quorum: raw voted balance must reach `circulating_supply * QUORUM_BPS`.
            let circulating_supply = self
                .election_snapshot
                .as_ref()
                .map(|s| s.circulating_supply)
                .unwrap_or(0);
            let quorum = circulating_supply
                .checked_mul(u128::from(QUORUM_BPS))
                .and_then(|x| x.checked_div(u128::from(BPS_DENOMINATOR)))
                .ok_or(Error::ArithmeticError)?;
            let quorum_met = self.total_voted_balance >= quorum;

            // Strict majority: best * 10_000 > total * 5_000.
            let lhs = best
                .checked_mul(u128::from(BPS_DENOMINATOR))
                .ok_or(Error::ArithmeticError)?;
            let rhs = self
                .total_voting_power
                .checked_mul(u128::from(APPROVAL_THRESHOLD_BPS))
                .ok_or(Error::ArithmeticError)?;
            let has_winner = quorum_met && best > 0 && lhs > rhs;

            if has_winner {
                let who = winner.ok_or(Error::NoWinner)?;
                let candidacy = self
                    .candidates
                    .get((self.cycle_id, who))
                    .ok_or(Error::CandidateNotFound)?;
                self.maintainer_elect = Some(MaintainerElect {
                    who,
                    netuid: candidacy.netuid,
                    approval_weight: best,
                    decided_at: now,
                });
                self.phase = Phase::Elected;
                self.env().emit_event(ElectionFinalized {
                    cycle_id: self.cycle_id,
                    winner: Some(who),
                    netuid: candidacy.netuid,
                    approval_weight: best,
                    total_voting_power: self.total_voting_power,
                });
            } else {
                // No winner: incumbent stays, advance the cadence to the next cycle.
                self.maintainer_elect = None;
                self.phase = Phase::Idle;
                self.advance_cadence()?;
                self.env().emit_event(ElectionFinalized {
                    cycle_id: self.cycle_id,
                    winner: None,
                    netuid: 0,
                    approval_weight: best,
                    total_voting_power: self.total_voting_power,
                });
            }
            Ok(())
        }

        /// Installs the elected maintainer (Elected → Idle). Permissionless; callable on/after the
        /// [`ACTIVATION_DAY`] (the 15th) — the winner is already decided, so anyone may push the
        /// installation through and the cycle can never stall on an absent winner. Installs the
        /// winner as governance's maintainer via the election-gated `elect_maintainer`; the new
        /// maintainer seats their own council on governance afterward. If the winner governs a
        /// different subnet, opens a 6-month transition during which the previous subnet stays
        /// authoritative and the netuid switch is deferred to [`end_transition`].
        #[ink(message)]
        pub fn activate(&mut self) -> Result<()> {
            if self.phase != Phase::Elected {
                return Err(Error::WrongPhase);
            }
            let elect = self.maintainer_elect.clone().ok_or(Error::NoWinner)?;
            let now = self.env().block_timestamp();
            if now < self.voting_ends_at {
                return Err(Error::VotingStillOpen);
            }
            if day_of_month(now) < ACTIVATION_DAY {
                return Err(Error::BeforeActivationDay);
            }

            // Install the winner as governance's maintainer. This contract is authorized as
            // governance's `election`; it never holds the maintainer role itself.
            self.gov_elect_maintainer(elect.who)?;

            // If the governing subnet changes, open (or reset) the 6-month transition. The netuid
            // switch itself is deferred to `end_transition`, so `active_netuid` stays the previous
            // subnet and governance's snapshot remains the electorate source meanwhile.
            if elect.netuid != self.active_netuid {
                let ends_at = now
                    .checked_add(TRANSITION_MS)
                    .ok_or(Error::ArithmeticError)?;
                self.transition = Some(Transition {
                    from_netuid: self.active_netuid,
                    to_netuid: elect.netuid,
                    ends_at,
                });
                self.env().emit_event(TransitionStarted {
                    from_netuid: self.active_netuid,
                    to_netuid: elect.netuid,
                    ends_at,
                });
            }

            self.incumbent = elect.who;
            let served = self.terms_served.get(elect.who).unwrap_or(0);
            self.terms_served.insert(
                elect.who,
                &served.checked_add(1).ok_or(Error::ArithmeticError)?,
            );
            self.advance_cadence()?;
            self.maintainer_elect = None;
            self.phase = Phase::Idle;

            self.env().emit_event(MaintainerActivated {
                cycle_id: self.cycle_id,
                new_maintainer: elect.who,
                active_netuid: self.active_netuid,
            });
            Ok(())
        }

        /// Completes a migration once its 6 months elapse: switches `active_netuid` to the new
        /// subnet and pushes the netuid change into governance. Permissionless.
        #[ink(message)]
        pub fn end_transition(&mut self) -> Result<()> {
            let transition = self.transition.clone().ok_or(Error::NoTransition)?;
            let now = self.env().block_timestamp();
            if now < transition.ends_at {
                return Err(Error::TransitionActive);
            }

            self.active_netuid = transition.to_netuid;
            self.gov_election_set_netuid(transition.to_netuid)?;
            self.transition = None;

            self.env().emit_event(TransitionEnded {
                netuid: transition.to_netuid,
            });
            Ok(())
        }

        /// Flags an emergency election (e.g. the governing subnet was deregistered);
        /// incumbent-only. The next `schedule_election` may then run immediately, bypassing the
        /// cadence anchor.
        #[ink(message)]
        pub fn trigger_emergency_election(&mut self) -> Result<()> {
            self.ensure_incumbent()?;
            if self.phase != Phase::Idle {
                return Err(Error::WrongPhase);
            }
            self.emergency_pending = true;
            self.env().emit_event(EmergencyElectionTriggered {
                triggered_by: self.env().caller(),
            });
            Ok(())
        }

        /// Cancels a stalled registration and returns to Idle; incumbent-only. Allowed only in
        /// `Registration` — the one phase that can genuinely stall (no first vote ever opens voting,
        /// so there is no window to finalize). A `Voting` cycle is always finalizable after its
        /// window, and an `Elected` cycle is resolved by the permissionless `activate`, so neither
        /// needs (nor should allow) cancellation — in particular this prevents an outgoing incumbent
        /// from vetoing a decided winner.
        #[ink(message)]
        pub fn cancel_cycle(&mut self) -> Result<()> {
            self.ensure_incumbent()?;
            if self.phase != Phase::Registration {
                return Err(Error::WrongPhase);
            }
            self.election_snapshot = None;
            self.candidate_list = Vec::new();
            self.total_voting_power = 0;
            self.total_voted_balance = 0;
            self.leading_candidate = None;
            self.leading_weight = 0;
            self.voting_opens_at = 0;
            self.voting_ends_at = 0;
            self.phase = Phase::Idle;
            Ok(())
        }

        // --------------------------------------------------------------------------------------- //
        // Internal helpers                                                                         //
        // --------------------------------------------------------------------------------------- //

        /// Resets per-cycle state under a fresh `cycle_id`, records the electorate `snapshot`, and
        /// enters `Registration`. Shared by `schedule_election` (which fetches the snapshot from
        /// governance) and the `#[cfg(test)]` `test_schedule` seam (which injects it).
        fn open_registration(
            &mut self,
            snapshot: ElectionSnapshot,
            now: Timestamp,
            emergency: bool,
        ) -> Result<()> {
            self.cycle_id = self.cycle_id.checked_add(1).ok_or(Error::ArithmeticError)?;
            self.candidate_list = Vec::new();
            self.total_voting_power = 0;
            self.total_voted_balance = 0;
            self.leading_candidate = None;
            self.leading_weight = 0;
            self.maintainer_elect = None;
            self.voting_opens_at = 0;
            self.voting_ends_at = 0;
            self.emergency_pending = false;
            self.election_snapshot = Some(snapshot);
            self.phase = Phase::Registration;
            self.env().emit_event(ElectionScheduled {
                cycle_id: self.cycle_id,
                emergency,
                scheduled_at: now,
            });
            Ok(())
        }

        /// Recomputes the next-election anchor from the genesis anchor and the (incremented) term
        /// index, so the 2-year cadence never accumulates millisecond drift.
        fn advance_cadence(&mut self) -> Result<()> {
            self.term_index = self
                .term_index
                .checked_add(1)
                .ok_or(Error::ArithmeticError)?;
            let offset = u64::from(self.term_index)
                .checked_mul(TERM_LENGTH_MS)
                .ok_or(Error::ArithmeticError)?;
            self.next_election_ts = self
                .genesis_election_ts
                .checked_add(offset)
                .ok_or(Error::ArithmeticError)?;
            Ok(())
        }

        fn ensure_incumbent(&self) -> Result<()> {
            if self.env().caller() != self.incumbent {
                return Err(Error::NotIncumbent);
            }
            Ok(())
        }

        /// Reads the `(hotkey, coldkey, netuid)` raw alpha stake via the chain extension.
        fn read_candidate_stake(
            &self,
            hotkey: AccountId,
            coldkey: AccountId,
            netuid: u16,
        ) -> Result<u128> {
            let info = self
                .env()
                .extension()
                .get_stake_info_for_hotkey_coldkey_netuid(hotkey, coldkey, netuid)
                .map_err(|_| Error::StakeQueryFailed)?
                .ok_or(Error::NoStake)?;
            Ok(u128::from(info.stake.0))
        }

        /// Raw cross-contract read of governance's latest snapshot, returned as
        /// `(root, circulating_supply, netuid, snapshot_block)`. Governance's `election_snapshot`
        /// returns `Option`, so `None` (no snapshot committed) maps to [`Error::NoSnapshot`].
        fn gov_latest_snapshot(&self) -> Result<(MerkleHash, u128, u16, u32)> {
            build_call::<tusdt_env::CustomEnvironment>()
                .call(self.governance)
                .exec_input(ExecutionInput::new(Selector::new(
                    SELECTOR_ELECTION_SNAPSHOT,
                )))
                .returns::<Option<(MerkleHash, u128, u16, u32)>>()
                .invoke()
                .ok_or(Error::NoSnapshot)
        }

        /// Raw cross-contract call to `governance.elect_maintainer(new_maintainer)`. Governance's
        /// `Error` is a fieldless enum, so its `Result<(), Error>` decodes layout-compatibly as
        /// `Result<(), u8>`; any callee failure is mapped to [`Error::GovernanceCallFailed`].
        fn gov_elect_maintainer(&self, new_maintainer: AccountId) -> Result<()> {
            build_call::<tusdt_env::CustomEnvironment>()
                .call(self.governance)
                .exec_input(
                    ExecutionInput::new(Selector::new(SELECTOR_ELECT_MAINTAINER))
                        .push_arg(new_maintainer),
                )
                .returns::<core::result::Result<(), u8>>()
                .invoke()
                .map_err(|_| Error::GovernanceCallFailed)
        }

        /// Raw cross-contract call to `governance.election_set_netuid(netuid)`; see
        /// [`gov_elect_maintainer`] for the return decoding rationale.
        fn gov_election_set_netuid(&self, netuid: u16) -> Result<()> {
            build_call::<tusdt_env::CustomEnvironment>()
                .call(self.governance)
                .exec_input(
                    ExecutionInput::new(Selector::new(SELECTOR_ELECTION_SET_NETUID))
                        .push_arg(netuid),
                )
                .returns::<core::result::Result<(), u8>>()
                .invoke()
                .map_err(|_| Error::GovernanceCallFailed)
        }

        // Test-only seams: `terms_served` and `transition` are otherwise only mutated inside
        // `activate`, which makes a live cross-contract call that off-chain `#[ink::test]` cannot
        // execute. These let unit tests exercise the term-limit and transition-timing gates
        // directly; they are compiled out of the deployed contract.
        /// Schedules a cycle with an injected electorate `snapshot`, replicating
        /// `schedule_election`'s phase/cadence/emergency gating but skipping the cross-contract read
        /// from governance (which off-chain `#[ink::test]` cannot perform).
        #[cfg(test)]
        pub(crate) fn test_schedule(&mut self, snapshot: ElectionSnapshot) -> Result<()> {
            if self.phase != Phase::Idle {
                return Err(Error::WrongPhase);
            }
            let now = self.env().block_timestamp();
            let emergency = self.emergency_pending;
            if !emergency && now < self.next_election_ts {
                return Err(Error::NotElectionTime);
            }
            self.open_registration(snapshot, now, emergency)
        }

        #[cfg(test)]
        pub(crate) fn test_set_terms_served(&mut self, who: AccountId, terms: u32) {
            self.terms_served.insert(who, &terms);
        }

        #[cfg(test)]
        pub(crate) fn test_set_transition(&mut self, transition: Transition) {
            self.transition = Some(transition);
        }
    }
}

#[cfg(test)]
mod tests;
