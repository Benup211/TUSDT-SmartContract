#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use self::governance::{
    CouncilSet, GovernanceParams, MaintainerChanged, Proposal, ProposalExecuted, ProposalFinalized,
    ProposalKind, ProposalStatus, ProposalSubmitted, Snapshot, SnapshotSubmitted, TusdtGovernance,
    TusdtGovernanceRef, Voted,
};

#[ink::contract(env = tusdt_env::CustomEnvironment)]
mod governance {
    use ink::prelude::string::String;
    use ink::prelude::vec::Vec;
    use ink::storage::Mapping;
    use ink::{env::call::FromAccountId, ToAccountId};
    use tusdt_auction::TusdtAuctionRef;
    use tusdt_election::TusdtElectionRef;
    use tusdt_oracle::{PriceData, TusdtOracleRef};
    use tusdt_primitives::Ratio;
    use tusdt_treasury::{Fund, TokenKind, TusdtTreasuryRef};
    use tusdt_vault::{TusdtVaultRef, VaultContractParamsConfig};

    // Cross-contract forwarders that let governance drive the vault, auction, and oracle.
    // `forward_*` helpers defined here.
    mod external_calls {
        include!("external_calls.rs");
    }

    // Snapshot/voting helpers (Merkle hashing/proofs, quadratic time-staked weighting, day-of-month)
    pub(crate) use tusdt_voting::*;

    /// Maximum CID byte length accepted by `submit_proposal` (CIDv1 base32 is ~62 chars).
    pub(crate) const MAX_CID_LEN: usize = 96;
    /// The council is a fixed-size committee; [`TusdtGovernance::set_council`] requires exactly
    /// this many distinct members.
    pub(crate) const COUNCIL_SIZE: usize = 5;
    pub(crate) const DEFAULT_NETUID: u16 = 421;
    pub(crate) const DEFAULT_VOTING_PERIOD_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
    pub(crate) const DEFAULT_APPROVAL_BPS: u32 = 5_001;
    /// Default quorum as a fraction of the alpha circulating supply, in basis points (2_000 = 20%).
    pub(crate) const DEFAULT_QUORUM_BPS: u32 = 2_000;
    pub(crate) const DEFAULT_MIN_PROPOSER_STAKE: u128 = 1_000_000_000_000;
    /// Proposals may only be submitted on these days of the month (inclusive), in UTC.
    pub(crate) const DEFAULT_SUBMISSION_OPEN_DAY: u8 = 20;
    pub(crate) const DEFAULT_SUBMISSION_CLOSE_DAY: u8 = 27;

    /// Tunable governance parameters. Updatable by the maintainer.
    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct GovernanceParams {
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
    #[allow(clippy::cast_possible_truncation)]
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
        /// Top authority: governs parameters and council membership. This is the *elected* account
        /// (the winning subnet owner), not a contract. It is replaced only by the election contract
        /// via [`TusdtGovernance::elect_maintainer`].
        maintainer: AccountId,
        /// The election contract. It is authorized to install the maintainer (`elect_maintainer`)
        /// and apply netuid switches (`election_set_netuid`).
        election: TusdtElectionRef,
        /// The operating committee, set by the maintainer via [`TusdtGovernance::set_council`].
        /// Holds exactly [`COUNCIL_SIZE`] members once seated (empty until the first call); council
        /// members perform operational duties such as committing snapshots.
        council: Vec<AccountId>,
        treasury: TusdtTreasuryRef,
        /// The vault, auction, and oracle this governance contract steers. After deployment the
        /// vault's governance role (and, via the vault's propagation, the auction's and oracle's)
        /// is handed to this contract, so these forwarding calls are accepted as `governance`.
        vault: TusdtVaultRef,
        auction: TusdtAuctionRef,
        oracle: TusdtOracleRef,
        /// The subnet whose alpha stake gates proposer eligibility. Bound to the elected maintainer
        /// (the subnet they govern) — only the election contract changes it, via [`TusdtGovernance::election_set_netuid`].
        netuid: u16,
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

    /// Emitted when the maintainer role is transferred (by the election contract).
    #[ink(event)]
    pub struct MaintainerChanged {
        #[ink(topic)]
        previous: AccountId,
        #[ink(topic)]
        new: AccountId,
    }

    /// Emitted when the maintainer (re)sets the council membership.
    #[ink(event)]
    pub struct CouncilSet {
        members: Vec<AccountId>,
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
        NotMaintainer,
        NotElection,
        NotCouncil,
        InvalidCouncil,
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
        VaultCallFailed,
        AuctionCallFailed,
        OracleCallFailed,
        ArithmeticError,
    }

    pub type Result<T> = core::result::Result<T, Error>;

    impl TusdtGovernance {
        /// Initializes governance with an explicit `maintainer`, references to the treasury, vault,
        /// auction, and oracle contracts, and default params. The maintainer is passed in (typically
        /// the subnet owner) rather than defaulting to the deployer.
        ///
        /// The election contract is **instantiated here** from `election_code_hash` and retained;
        /// it is authorized to install maintainers (`elect_maintainer`) and apply netuid switches
        /// (`election_set_netuid`). The initial maintainer is also seated as the election's initial
        /// incumbent. The election derives its genesis cadence anchor from the deployment block
        /// timestamp and its candidate stake bar from its own constant. The council starts empty and
        /// must be seated by the maintainer via [`set_council`].
        ///
        /// The vault/auction/oracle addresses are wired so governance can forward privileged calls
        /// to them once it holds their `governance` role (see [`external_calls`]).
        #[ink(constructor)]
        pub fn new(
            treasury_address: AccountId,
            vault_address: AccountId,
            auction_address: AccountId,
            oracle_address: AccountId,
            maintainer: AccountId,
            election_code_hash: Hash,
        ) -> Self {
            // Instantiate the election, handing it this contract's address so it can call back into
            // `elect_maintainer` / `election_set_netuid` and read snapshots. The initial maintainer
            // is its incumbent, governing the default subnet.
            let election =
                TusdtElectionRef::new(Self::env().account_id(), maintainer, DEFAULT_NETUID)
                    .code_hash(election_code_hash)
                    .endowment(0)
                    .salt_bytes([0u8; 32])
                    .instantiate();
            Self::from_addresses(
                treasury_address,
                vault_address,
                auction_address,
                oracle_address,
                maintainer,
                election.to_account_id(),
            )
        }

        /// Builds the storage struct from the addresses of the already-deployed peer contracts. Used
        /// by [`new`] (after it instantiates the election) and by unit tests (which can't instantiate
        /// cross-contracts, so they pass a stand-in `election` address that `ensure_election` checks
        /// against). All refs are reconstructed from their `AccountId` via `FromAccountId`.
        pub(crate) fn from_addresses(
            treasury_address: AccountId,
            vault_address: AccountId,
            auction_address: AccountId,
            oracle_address: AccountId,
            maintainer: AccountId,
            election: AccountId,
        ) -> Self {
            Self {
                maintainer,
                election: TusdtElectionRef::from_account_id(election),
                council: Vec::new(),
                treasury: TusdtTreasuryRef::from_account_id(treasury_address),
                vault: TusdtVaultRef::from_account_id(vault_address),
                auction: TusdtAuctionRef::from_account_id(auction_address),
                oracle: TusdtOracleRef::from_account_id(oracle_address),
                netuid: DEFAULT_NETUID,
                params: GovernanceParams::default_params(),
                current_epoch: 0,
                snapshots: Mapping::default(),
                proposal_count: 0,
                proposals: Mapping::default(),
                has_voted: Mapping::default(),
            }
        }

        /// Returns the maintainer account.
        #[ink(message)]
        pub fn maintainer(&self) -> AccountId {
            self.maintainer
        }

        /// Returns the governing subnet netuid (bound to the maintainer; set only by the election).
        #[ink(message)]
        pub fn netuid(&self) -> u16 {
            self.netuid
        }

        /// Returns the election contract address (instantiated at construction).
        #[ink(message)]
        pub fn election(&self) -> AccountId {
            self.election.to_account_id()
        }

        /// Returns the current council members (empty until the maintainer seats them).
        #[ink(message)]
        pub fn council(&self) -> Vec<AccountId> {
            self.council.clone()
        }

        /// Reports whether `who` is currently a council member.
        #[ink(message)]
        pub fn is_council(&self, who: AccountId) -> bool {
            self.council.contains(&who)
        }

        /// Returns the treasury contract address.
        #[ink(message)]
        pub fn treasury(&self) -> AccountId {
            self.treasury.to_account_id()
        }

        /// Installs `new_maintainer` as the maintainer; callable only by the election contract.
        /// The explicit selector is matched by the election's raw cross-contract call.
        #[ink(message, selector = 0xE1EC7000)]
        pub fn elect_maintainer(&mut self, new_maintainer: AccountId) -> Result<()> {
            self.ensure_election()?;
            let previous = self.maintainer;
            self.maintainer = new_maintainer;
            self.env().emit_event(MaintainerChanged {
                previous,
                new: new_maintainer,
            });
            Ok(())
        }

        /// Sets the governing subnet `netuid`; callable only by the election contract (used when a
        /// cross-subnet election transition completes). The explicit selector is matched by the
        /// election's raw cross-contract call.
        #[ink(message, selector = 0xE1EC7001)]
        pub fn election_set_netuid(&mut self, netuid: u16) -> Result<()> {
            self.ensure_election()?;
            self.netuid = netuid;
            Ok(())
        }

        /// Replaces the entire council with `members`; maintainer-only. Requires exactly
        /// [`COUNCIL_SIZE`] distinct members, otherwise returns [`Error::InvalidCouncil`].
        #[ink(message)]
        pub fn set_council(&mut self, members: Vec<AccountId>) -> Result<()> {
            self.ensure_maintainer()?;
            if members.len() != COUNCIL_SIZE {
                return Err(Error::InvalidCouncil);
            }
            // Reject duplicates so the committee really has COUNCIL_SIZE distinct members.
            for (i, m) in members.iter().enumerate() {
                if members
                    .iter()
                    .skip(i.saturating_add(1))
                    .any(|other| other == m)
                {
                    return Err(Error::InvalidCouncil);
                }
            }
            self.council = members.clone();
            self.env().emit_event(CouncilSet { members });
            Ok(())
        }

        // ---------------------------------------------------------------------------------------
        // Cross-contract forwarders. Each is a thin `#[ink(message)]` entry point delegating to a
        // `forward_*` helper in `external_calls.rs`, where the authorization and the actual
        // cross-contract call live. Maintainer-gated calls are governing/config actions (plus the
        // emergency oracle price); `vault_pause` is the one council-gated
        // operational call, kept fast for emergencies.
        // ---------------------------------------------------------------------------------------

        /// Schedules a vault contract-parameter update (behind the vault's timelock); maintainer-only.
        #[ink(message)]
        pub fn vault_set_contract_params(
            &mut self,
            params: VaultContractParamsConfig,
        ) -> Result<()> {
            self.forward_vault_set_contract_params(params)
        }

        /// Cancels the vault's currently scheduled contract-parameter update; maintainer-only.
        #[ink(message)]
        pub fn vault_cancel_contract_params_update(&mut self) -> Result<()> {
            self.forward_vault_cancel_contract_params_update()
        }

        /// Updates the vault's treasury (fee recipient) account; maintainer-only.
        #[ink(message)]
        pub fn vault_update_treasury(&mut self, new_treasury: AccountId) -> Result<()> {
            self.forward_vault_update_treasury(new_treasury)
        }

        /// Updates the vault's platform (pause operator) account; maintainer-only.
        #[ink(message)]
        pub fn vault_update_platform(&mut self, new_platform: AccountId) -> Result<()> {
            self.forward_vault_update_platform(new_platform)
        }

        /// Unpauses the vault; maintainer-only (deliberate recovery).
        #[ink(message)]
        pub fn vault_unpause(&mut self) -> Result<()> {
            self.forward_vault_unpause()
        }

        /// Pauses the vault; council-only (operational/emergency halt).
        #[ink(message)]
        pub fn vault_pause(&mut self) -> Result<()> {
            self.forward_vault_pause()
        }

        /// Sets/clears the oracle's round-committing validator; maintainer-only.
        #[ink(message)]
        pub fn oracle_set_validator(&mut self, validator: Option<AccountId>) -> Result<()> {
            self.forward_oracle_set_validator(validator)
        }

        /// Updates the oracle's max allowed price deviation; maintainer-only.
        #[ink(message)]
        pub fn oracle_set_max_price_deviation(&mut self, max_price_deviation: Ratio) -> Result<()> {
            self.forward_oracle_set_max_price_deviation(max_price_deviation)
        }

        /// Commits an emergency oracle price, bypassing quorum/deviation checks; maintainer-only
        #[ink(message)]
        pub fn oracle_commit_round(&mut self, price: Ratio) -> Result<PriceData> {
            self.forward_oracle_commit_round(price)
        }

        /// Updates the governing subnet netuid for the oracle; maintainer-only.
        #[ink(message)]
        pub fn oracle_set_netuid(&mut self, netuid: u16) -> Result<()> {
            self.forward_oracle_set_netuid(netuid)
        }

        /// Updates the minimum submitter stake threshold for the oracle; maintainer-only.
        #[ink(message)]
        pub fn oracle_set_min_submitter_stake(&mut self, min_stake: u128) -> Result<()> {
            self.forward_oracle_set_min_submitter_stake(min_stake)
        }

        /// Sets/clears the auction admin (allowed to bid on expired no-bid auctions); maintainer-only.
        #[ink(message)]
        pub fn auction_set_admin(&mut self, admin: Option<AccountId>) -> Result<()> {
            self.forward_auction_set_admin(admin)
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

        /// Returns the latest committed snapshot packaged for the election contract as
        /// `(root, circulating_supply, netuid, snapshot_block)`, or `None` if none committed yet.
        /// The explicit selector is matched by the election's raw cross-contract call.
        #[ink(message, selector = 0xE1EC7002)]
        pub fn election_snapshot(&self) -> Option<(MerkleHash, u128, u16, u32)> {
            self.snapshots
                .get(self.current_epoch)
                .map(|s| (s.root, s.circulating_supply, self.netuid, s.snapshot_block))
        }

        /// Commits a new off-chain snapshot and returns its epoch; council-only (operational duty).
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
            self.ensure_council()?;
            let epoch = self
                .current_epoch
                .checked_add(1)
                .ok_or(Error::ArithmeticError)?;
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
                .and_then(|s| {
                    s.circulating_supply
                        .saturating_mul(u128::from(self.params.quorum_bps))
                        .checked_div(u128::from(BPS_DENOMINATOR))
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

        /// Updates the governance parameters; maintainer-only.
        #[ink(message)]
        pub fn update_params(&mut self, new_params: GovernanceParams) -> Result<()> {
            self.ensure_maintainer()?;
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

        /// Submits a new proposal. Only council members may submit. Proposals are
        /// accepted during the configured day-of-month window (UTC, inclusive).
        #[ink(message)]
        pub fn submit_proposal(
            &mut self,
            cid: String,
            kind: ProposalKind,
        ) -> Result<u64> {
            // Only council members may submit proposals.
            self.ensure_council()?;

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

            let weight = voting_power(balance, multiplier_bps).ok_or(Error::ArithmeticError)?;
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
                    .checked_mul(u128::from(BPS_DENOMINATOR))
                    .and_then(|x| x.checked_div(total))
                    .ok_or(Error::ArithmeticError)?;
                if yes_bps >= u128::from(self.params.approval_bps) {
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

        fn ensure_maintainer(&self) -> Result<()> {
            if self.env().caller() != self.maintainer {
                return Err(Error::NotMaintainer);
            }
            Ok(())
        }

        fn ensure_election(&self) -> Result<()> {
            if self.env().caller() != self.election.to_account_id() {
                return Err(Error::NotElection);
            }
            Ok(())
        }

        fn ensure_council(&self) -> Result<()> {
            if !self.council.contains(&self.env().caller()) {
                return Err(Error::NotCouncil);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
