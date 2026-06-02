#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use self::governance::{
    GovernanceParams, Proposal, ProposalExecuted, ProposalFinalized, ProposalKind, ProposalStatus,
    ProposalSubmitted, Snapshot, SnapshotSubmitted, TusdtGovernance, TusdtGovernanceRef, Voted,
};

#[ink::contract(env = tusdt_env::CustomEnvironment)]
mod governance {
    use ink::prelude::string::String;
    use ink::prelude::vec::Vec;
    use ink::storage::Mapping;
    use ink::{env::call::FromAccountId, ToAccountId};
    use tusdt_treasury::{Fund, TokenKind, TusdtTreasuryRef};

    /// A 32-byte Blake2b-256 digest used for Merkle roots, leaves, and proof nodes.
    pub(crate) type MerkleHash = [u8; 32];

    /// Maximum CID byte length accepted by `submit_proposal` (CIDv1 base32 is ~62 chars).
    pub(crate) const MAX_CID_LEN: usize = 96;
    pub(crate) const BPS_DENOMINATOR: u32 = 10_000;
    pub(crate) const DEFAULT_NETUID: u16 = 113;
    pub(crate) const DEFAULT_VOTING_PERIOD_MS: u64 = 48 * 60 * 60 * 1_000;
    pub(crate) const DEFAULT_APPROVAL_BPS: u32 = 5_001;
    /// Default quorum as a fraction of the alpha circulating supply, in basis points (2_000 = 20%).
    pub(crate) const DEFAULT_QUORUM_BPS: u32 = 2_000;
    pub(crate) const DEFAULT_MIN_PROPOSER_STAKE: u128 = 1_000_000_000_000;
    /// Proposals may only be submitted on these days of the month (inclusive), in UTC.
    pub(crate) const DEFAULT_SUBMISSION_OPEN_DAY: u8 = 5;
    pub(crate) const DEFAULT_SUBMISSION_CLOSE_DAY: u8 = 25;
    /// Milliseconds in a day; block timestamps are Unix epoch milliseconds.
    pub(crate) const MS_PER_DAY: u64 = 86_400_000;

    /// Tunable governance parameters. Updatable by admin.
    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct GovernanceParams {
        pub netuid: u16,
        pub voting_period_ms: u64,
        /// Quorum as a fraction of the alpha circulating supply, in basis points (2_000 = 20%).
        /// A proposal passes only if the raw balance that voted reaches `circulating_supply *
        /// quorum_bps / 10_000`; see [`quorum`].
        pub quorum_bps: u32,
        pub approval_bps: u32,
        pub min_proposer_stake: u128,
        /// First day-of-month (UTC, inclusive) on which proposals may be submitted.
        pub submission_open_day: u8,
        /// Last day-of-month (UTC, inclusive) on which proposals may be submitted.
        pub submission_close_day: u8,
    }

    impl GovernanceParams {
        pub fn default_params() -> Self {
            Self {
                netuid: DEFAULT_NETUID,
                voting_period_ms: DEFAULT_VOTING_PERIOD_MS,
                quorum_bps: DEFAULT_QUORUM_BPS,
                approval_bps: DEFAULT_APPROVAL_BPS,
                min_proposer_stake: DEFAULT_MIN_PROPOSER_STAKE,
                submission_open_day: DEFAULT_SUBMISSION_OPEN_DAY,
                submission_close_day: DEFAULT_SUBMISSION_CLOSE_DAY,
            }
        }
    }

    /// Funding proposals release `amount` of `token_kind` from `fund` to `recipient` on execution.
    /// NonFunding proposals are signal-only.
    #[derive(Debug, Clone)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub enum ProposalKind {
        Funding {
            fund: Fund,
            token_kind: TokenKind,
            amount: Balance,
            recipient: AccountId,
        },
        NonFunding,
    }

    /// Lifecycle stage of a proposal.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub enum ProposalStatus {
        Active,
        Passed,
        Rejected,
        Executed,
    }

    /// On-chain proposal record. `cid` points to the off-chain proposal document.
    #[derive(Debug, Clone)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct Proposal {
        pub id: u64,
        pub proposer: AccountId,
        pub cid: String,
        pub kind: ProposalKind,
        pub created_at: Timestamp,
        pub voting_ends_at: Timestamp,
        /// Snapshot epoch this proposal is bound to; votes prove membership against its root.
        pub snapshot_epoch: u64,
        /// Accumulated voting power in favor (see [`Voted::weight`]).
        pub yes: u128,
        /// Accumulated voting power against.
        pub no: u128,
        /// Sum of raw (snapshot-frozen) alpha balance that has voted; measured against the quorum.
        pub voted_balance: u128,
        pub status: ProposalStatus,
    }

    /// An off-chain governance snapshot committed on-chain. The Merkle tree's leaves are
    /// `blake2_256(SCALE(coldkey, hotkey, balance, multiplier_bps))`, with internal nodes hashing
    /// the sorted pair of children. The balance is frozen here, so flash-staking cannot inflate
    /// voting power, and the time-staked `multiplier_bps` is carried per leaf.
    #[derive(Debug, Clone)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct Snapshot {
        /// Merkle root over all eligible `(coldkey, hotkey, balance, multiplier_bps)` leaves.
        pub root: MerkleHash,
        /// Alpha circulating supply at the snapshot block; the base for the quorum.
        pub circulating_supply: u128,
        /// Subnet block height the snapshot was taken at (for off-chain auditing).
        pub snapshot_block: u32,
    }

    /// Governance storage.
    #[ink(storage)]
    pub struct TusdtGovernance {
        // TODO: replace single-admin role with a maintainer and councils.
        admin: AccountId,
        treasury: TusdtTreasuryRef,
        params: GovernanceParams,
        // Latest committed snapshot epoch; 0 means no snapshot has been submitted yet.
        current_epoch: u64,
        // Per-epoch snapshots; proposals bind to one at submission and votes prove against it.
        snapshots: Mapping<u64, Snapshot>,
        proposal_count: u64,
        proposals: Mapping<u64, Proposal>,
        // Composite key `(proposal_id, coldkey, hotkey) -> ()` blocks double votes on the same pair.
        has_voted: Mapping<(u64, AccountId, AccountId), ()>,
    }

    /// Emitted when a proposal is created.
    #[ink(event)]
    pub struct ProposalSubmitted {
        #[ink(topic)]
        proposal_id: u64,
        #[ink(topic)]
        proposer: AccountId,
        voting_ends_at: Timestamp,
    }

    /// Emitted on each cast vote with the voting power contributed.
    #[ink(event)]
    pub struct Voted {
        #[ink(topic)]
        proposal_id: u64,
        #[ink(topic)]
        coldkey: AccountId,
        hotkey: AccountId,
        support: bool,
        /// Voting power added to the tally: `sqrt(SN113 alpha) * time-staked multiplier`.
        weight: u128,
    }

    /// Emitted when voting closes and the outcome is decided.
    #[ink(event)]
    pub struct ProposalFinalized {
        #[ink(topic)]
        proposal_id: u64,
        status: ProposalStatus,
        yes: u128,
        no: u128,
    }

    /// Emitted when a passed proposal is executed; for NonFunding the treasury call is skipped.
    #[ink(event)]
    pub struct ProposalExecuted {
        #[ink(topic)]
        proposal_id: u64,
    }

    /// Emitted when a new off-chain snapshot is committed on-chain.
    #[ink(event)]
    pub struct SnapshotSubmitted {
        #[ink(topic)]
        epoch: u64,
        root: MerkleHash,
        circulating_supply: u128,
        snapshot_block: u32,
    }

    /// Errors returned by the governance contract.
    #[derive(Debug, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    pub enum Error {
        NotAdmin,
        ProposalNotFound,
        ProposalNotActive,
        VotingClosed,
        VotingStillOpen,
        AlreadyVoted,
        AlreadyExecuted,
        NoStake,
        InsufficientStake,
        OutsideSubmissionWindow,
        NoSnapshot,
        InvalidProof,
        StakeQueryFailed,
        InvalidCid,
        InvalidAmount,
        InvalidParams,
        NotPassed,
        TreasuryCallFailed,
        ArithmeticError,
    }

    pub type Result<T> = core::result::Result<T, Error>;

    impl TusdtGovernance {
        /// Initializes governance with an admin (deployer), a treasury reference, and default params.
        #[ink(constructor)]
        pub fn new(treasury_address: AccountId) -> Self {
            let treasury = TusdtTreasuryRef::from_account_id(treasury_address);
            Self {
                admin: Self::env().caller(),
                treasury,
                params: GovernanceParams::default_params(),
                current_epoch: 0,
                snapshots: Mapping::default(),
                proposal_count: 0,
                proposals: Mapping::default(),
                has_voted: Mapping::default(),
            }
        }

        /// Returns the admin account.
        #[ink(message)]
        pub fn admin(&self) -> AccountId {
            self.admin
        }

        /// Returns the treasury contract address.
        #[ink(message)]
        pub fn treasury(&self) -> AccountId {
            self.treasury.to_account_id()
        }

        /// Returns the current governance parameters.
        #[ink(message)]
        pub fn params(&self) -> GovernanceParams {
            self.params
        }

        /// Returns the latest committed snapshot epoch (0 if none submitted yet).
        #[ink(message)]
        pub fn current_epoch(&self) -> u64 {
            self.current_epoch
        }

        /// Looks up a snapshot by epoch.
        #[ink(message)]
        pub fn get_snapshot(&self, epoch: u64) -> Option<Snapshot> {
            self.snapshots.get(epoch)
        }

        /// Commits a new off-chain snapshot and returns its epoch; admin-only.
        ///
        /// `root` is the Merkle root over `(coldkey, hotkey, balance, multiplier_bps)` leaves (see
        /// [`Snapshot`]); `circulating_supply` is the alpha supply that the quorum is derived from;
        /// `snapshot_block` records the subnet height for off-chain auditing. Each call advances the
        /// epoch by one; proposals submitted afterward bind to the new epoch.
        #[ink(message)]
        pub fn submit_snapshot(
            &mut self,
            root: MerkleHash,
            circulating_supply: u128,
            snapshot_block: u32,
        ) -> Result<u64> {
            self.ensure_admin()?;
            let epoch = self.current_epoch.checked_add(1).ok_or(Error::ArithmeticError)?;
            self.snapshots.insert(
                epoch,
                &Snapshot {
                    root,
                    circulating_supply,
                    snapshot_block,
                },
            );
            self.current_epoch = epoch;
            self.env().emit_event(SnapshotSubmitted {
                epoch,
                root,
                circulating_supply,
                snapshot_block,
            });
            Ok(epoch)
        }

        /// Returns the absolute quorum threshold for `epoch`: `circulating_supply * quorum_bps /
        /// 10_000`. A proposal must gather at least this much raw voted balance (see
        /// [`Proposal::voted_balance`]) to be eligible to pass. Returns 0 if the epoch has no
        /// snapshot.
        #[ink(message)]
        pub fn quorum(&self, epoch: u64) -> u128 {
            self.snapshots
                .get(epoch)
                .map(|s| {
                    s.circulating_supply
                        .saturating_mul(self.params.quorum_bps as u128)
                        / BPS_DENOMINATOR as u128
                })
                .unwrap_or(0)
        }

        /// Returns the total number of proposals submitted.
        #[ink(message)]
        pub fn proposal_count(&self) -> u64 {
            self.proposal_count
        }

        /// Looks up a proposal by id.
        #[ink(message)]
        pub fn get_proposal(&self, proposal_id: u64) -> Option<Proposal> {
            self.proposals.get(proposal_id)
        }

        /// Reports whether `(coldkey, hotkey)` has already voted on `proposal_id`.
        #[ink(message)]
        pub fn has_voted(&self, proposal_id: u64, coldkey: AccountId, hotkey: AccountId) -> bool {
            self.has_voted.contains((proposal_id, coldkey, hotkey))
        }

        /// Updates the governance parameters; admin-only.
        #[ink(message)]
        pub fn update_params(&mut self, new_params: GovernanceParams) -> Result<()> {
            self.ensure_admin()?;
            if new_params.approval_bps == 0 || new_params.approval_bps > BPS_DENOMINATOR {
                return Err(Error::InvalidParams);
            }
            if new_params.quorum_bps > BPS_DENOMINATOR {
                return Err(Error::InvalidParams);
            }
            if new_params.voting_period_ms == 0 {
                return Err(Error::InvalidParams);
            }
            // Window must be a valid, non-empty range of real day-of-month values. Capped at 28 so
            // the window is reachable in every month, including February.
            if new_params.submission_open_day < 1
                || new_params.submission_open_day > new_params.submission_close_day
                || new_params.submission_close_day > 28
            {
                return Err(Error::InvalidParams);
            }
            self.params = new_params;
            Ok(())
        }

        /// Submits a new proposal. The caller is treated as the coldkey; `hotkey` is provided
        /// explicitly and the pair's subnet alpha stake must exceed `MIN_PROPOSER_ALPHA_STAKE`.
        #[ink(message)]
        pub fn submit_proposal(
            &mut self,
            cid: String,
            kind: ProposalKind,
            hotkey: AccountId,
        ) -> Result<u64> {
            if cid.is_empty() || cid.len() > MAX_CID_LEN {
                return Err(Error::InvalidCid);
            }
            if let ProposalKind::Funding { amount, .. } = &kind {
                if *amount == 0 {
                    return Err(Error::InvalidAmount);
                }
            }

            // Only accept submissions within the open day-of-month window (UTC).
            let now = self.env().block_timestamp();
            let day = day_of_month(now);
            if day < self.params.submission_open_day || day > self.params.submission_close_day {
                return Err(Error::OutsideSubmissionWindow);
            }

            // Bind the proposal to the current snapshot so in-flight votes prove against a fixed
            // root even if a newer snapshot lands during the voting period.
            let snapshot_epoch = self.current_epoch;
            if snapshot_epoch == 0 {
                return Err(Error::NoSnapshot);
            }

            let proposer = self.env().caller();

            // Gate submission on the proposer's subnet alpha stake.
            let stake = self.read_stake_weight(hotkey, proposer)?;
            if stake <= self.params.min_proposer_stake {
                return Err(Error::InsufficientStake);
            }

            let id = self
                .proposal_count
                .checked_add(1)
                .ok_or(Error::ArithmeticError)?;
            let voting_ends_at = now
                .checked_add(self.params.voting_period_ms)
                .ok_or(Error::ArithmeticError)?;

            let proposal = Proposal {
                id,
                proposer,
                cid,
                kind,
                created_at: now,
                voting_ends_at,
                snapshot_epoch,
                yes: 0,
                no: 0,
                voted_balance: 0,
                status: ProposalStatus::Active,
            };
            self.proposals.insert(id, &proposal);
            self.proposal_count = id;

            self.env().emit_event(ProposalSubmitted {
                proposal_id: id,
                proposer,
                voting_ends_at,
            });

            Ok(id)
        }

        /// Casts a vote on `proposal_id`. The caller is the coldkey; `hotkey`, `balance`, and
        /// `multiplier_bps` are the caller's leaf in the proposal's snapshot, proven by `proof`.
        ///
        /// Voting power is `sqrt(balance) * multiplier_bps / 10_000`, where `balance` is the
        /// snapshot-frozen SN113 alpha and `multiplier_bps` is the time-staked multiplier.
        #[ink(message)]
        pub fn vote(
            &mut self,
            proposal_id: u64,
            hotkey: AccountId,
            support: bool,
            balance: u128,
            multiplier_bps: u32,
            proof: Vec<MerkleHash>,
        ) -> Result<()> {
            let mut proposal = self
                .proposals
                .get(proposal_id)
                .ok_or(Error::ProposalNotFound)?;
            if proposal.status != ProposalStatus::Active {
                return Err(Error::ProposalNotActive);
            }
            let now = self.env().block_timestamp();
            if now >= proposal.voting_ends_at {
                return Err(Error::VotingClosed);
            }

            let coldkey = self.env().caller();
            let key = (proposal_id, coldkey, hotkey);
            if self.has_voted.contains(key) {
                return Err(Error::AlreadyVoted);
            }

            // Verify the caller's leaf against the snapshot the proposal is bound to.
            let snapshot = self
                .snapshots
                .get(proposal.snapshot_epoch)
                .ok_or(Error::NoSnapshot)?;
            let leaf = leaf_hash(coldkey, hotkey, balance, multiplier_bps);
            if !verify_merkle_proof(&proof, snapshot.root, leaf) {
                return Err(Error::InvalidProof);
            }

            let weight = voting_power(balance, multiplier_bps)?;
            if weight == 0 {
                return Err(Error::NoStake);
            }

            if support {
                proposal.yes = proposal
                    .yes
                    .checked_add(weight)
                    .ok_or(Error::ArithmeticError)?;
            } else {
                proposal.no = proposal
                    .no
                    .checked_add(weight)
                    .ok_or(Error::ArithmeticError)?;
            }
            // Track raw balance participation for the quorum (in circulating-supply units).
            proposal.voted_balance = proposal
                .voted_balance
                .checked_add(balance)
                .ok_or(Error::ArithmeticError)?;
            self.proposals.insert(proposal_id, &proposal);
            self.has_voted.insert(key, &());

            self.env().emit_event(Voted {
                proposal_id,
                coldkey,
                hotkey,
                support,
                weight,
            });
            Ok(())
        }

        /// Closes voting and decides the outcome. Permissionless; only callable after `voting_ends_at`.
        #[ink(message)]
        pub fn finalize(&mut self, proposal_id: u64) -> Result<()> {
            let mut proposal = self
                .proposals
                .get(proposal_id)
                .ok_or(Error::ProposalNotFound)?;
            if proposal.status != ProposalStatus::Active {
                return Err(Error::ProposalNotActive);
            }
            let now = self.env().block_timestamp();
            if now < proposal.voting_ends_at {
                return Err(Error::VotingStillOpen);
            }

            // Quorum is measured by raw balance that voted (same units as circulating supply);
            // the approval ratio is weighted by voting power (yes / (yes + no)).
            let total = proposal
                .yes
                .checked_add(proposal.no)
                .ok_or(Error::ArithmeticError)?;
            let quorum_met = proposal.voted_balance >= self.quorum(proposal.snapshot_epoch);
            let new_status = if !quorum_met || total == 0 {
                ProposalStatus::Rejected
            } else {
                let yes_bps = proposal
                    .yes
                    .checked_mul(BPS_DENOMINATOR as u128)
                    .ok_or(Error::ArithmeticError)?
                    / total;
                if yes_bps >= self.params.approval_bps as u128 {
                    ProposalStatus::Passed
                } else {
                    ProposalStatus::Rejected
                }
            };

            proposal.status = new_status;
            let yes = proposal.yes;
            let no = proposal.no;
            self.proposals.insert(proposal_id, &proposal);

            self.env().emit_event(ProposalFinalized {
                proposal_id,
                status: new_status,
                yes,
                no,
            });
            Ok(())
        }

        /// Executes a passed proposal. For Funding, calls `treasury.release(...)`. Permissionless.
        #[ink(message)]
        pub fn execute(&mut self, proposal_id: u64) -> Result<()> {
            let mut proposal = self
                .proposals
                .get(proposal_id)
                .ok_or(Error::ProposalNotFound)?;
            match proposal.status {
                ProposalStatus::Passed => {}
                ProposalStatus::Executed => return Err(Error::AlreadyExecuted),
                _ => return Err(Error::NotPassed),
            }

            if let ProposalKind::Funding {
                fund,
                token_kind,
                amount,
                recipient,
            } = proposal.kind.clone()
            {
                self.treasury
                    .release(fund, token_kind, amount, recipient)
                    .map_err(|_| Error::TreasuryCallFailed)?;
            }

            proposal.status = ProposalStatus::Executed;
            self.proposals.insert(proposal_id, &proposal);

            self.env().emit_event(ProposalExecuted { proposal_id });
            Ok(())
        }

        /// Calls the chain extension to fetch the (hotkey, coldkey, netuid) raw alpha stake.
        fn read_stake_weight(&self, hotkey: AccountId, coldkey: AccountId) -> Result<u128> {
            let info = self
                .env()
                .extension()
                .get_stake_info_for_hotkey_coldkey_netuid(hotkey, coldkey, self.params.netuid)
                .map_err(|_| Error::StakeQueryFailed)?
                .ok_or(Error::NoStake)?;
            Ok(info.stake.0 as u128)
        }

        fn ensure_admin(&self) -> Result<()> {
            if self.env().caller() != self.admin {
                return Err(Error::NotAdmin);
            }
            Ok(())
        }
    }

    /// Returns the UTC day-of-month (1..=31) for a Unix-epoch timestamp in milliseconds.
    ///
    /// Uses Howard Hinnant's `civil_from_days` algorithm. Timestamps are always post-1970, so the
    /// day count is non-negative and the negative-era branch of the general algorithm is omitted.
    pub(crate) fn day_of_month(timestamp_ms: u64) -> u8 {
        // Days since the 1970-01-01 epoch.
        let days = (timestamp_ms / MS_PER_DAY) as i64;
        // Shift the era origin to 0000-03-01 so leap days fall at the end of the 400-year cycle.
        let z = days + 719_468;
        let era = z / 146_097;
        let doe = z - era * 146_097; // day-of-era, [0, 146096]
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year-of-era, [0, 399]
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day-of-year (Mar-based), [0, 365]
        let mp = (5 * doy + 2) / 153; // month, Mar=0..Feb=11
        let day = doy - (153 * mp + 2) / 5 + 1; // day-of-month, [1, 31]
        day as u8
    }

    /// Integer (floor) square root of `n`, via Newton's method. No floats are available on-chain.
    pub(crate) fn integer_sqrt(n: u128) -> u128 {
        if n < 2 {
            return n;
        }
        let mut x = n;
        let mut y = (x + 1) / 2;
        while y < x {
            x = y;
            y = (x + n / x) / 2;
        }
        x
    }

    /// Voting power for a snapshot leaf: `sqrt(balance) * multiplier_bps / 10_000`.
    /// The square root dampens large stakes; the time-staked `multiplier_bps` rewards longer-held
    /// stake (0.5x = 5_000, 0.8x = 8_000, 1.0x = 10_000).
    pub(crate) fn voting_power(balance: u128, multiplier_bps: u32) -> Result<u128> {
        integer_sqrt(balance)
            .checked_mul(multiplier_bps as u128)
            .map(|scaled| scaled / BPS_DENOMINATOR as u128)
            .ok_or(Error::ArithmeticError)
    }

    /// Computes a snapshot leaf hash: `blake2_256(SCALE(coldkey, hotkey, balance, multiplier_bps))`.
    /// Off-chain proof generation must encode and hash the tuple identically.
    pub(crate) fn leaf_hash(
        coldkey: AccountId,
        hotkey: AccountId,
        balance: u128,
        multiplier_bps: u32,
    ) -> MerkleHash {
        let mut out = MerkleHash::default();
        ink::env::hash_encoded::<ink::env::hash::Blake2x256, _>(
            &(coldkey, hotkey, balance, multiplier_bps),
            &mut out,
        );
        out
    }

    /// Hashes a pair of sibling nodes in sorted order: `blake2_256(min(a,b) ++ max(a,b))`.
    /// Sorting lets proofs omit per-node position flags (OpenZeppelin convention).
    pub(crate) fn hash_pair(a: MerkleHash, b: MerkleHash) -> MerkleHash {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let mut input = [0u8; 64];
        input[..32].copy_from_slice(&lo);
        input[32..].copy_from_slice(&hi);
        let mut out = MerkleHash::default();
        ink::env::hash_bytes::<ink::env::hash::Blake2x256>(&input, &mut out);
        out
    }

    /// Verifies that `leaf` is part of the tree with the given `root`, folding `proof` bottom-up.
    pub(crate) fn verify_merkle_proof(proof: &[MerkleHash], root: MerkleHash, leaf: MerkleHash) -> bool {
        let mut computed = leaf;
        for sibling in proof {
            computed = hash_pair(computed, *sibling);
        }
        computed == root
    }
}

#[cfg(test)]
mod tests;
