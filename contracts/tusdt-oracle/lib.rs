#![cfg_attr(not(feature = "std"), no_std, no_main)]
#![allow(clippy::enum_variant_names)]

pub use self::oracle::{
    PriceData, PriceSubmission, PriceSubmissionMetadata, RoundSummary, TusdtOracle, TusdtOracleRef,
};

#[ink::contract(env = tusdt_env::CustomEnvironment)]
mod oracle {
    use core::cmp::min;
    use ink::{prelude::vec::Vec, storage::Mapping};
    use tusdt_primitives::Ratio;

    const MIN_REPORTERS: u32 = 3;
    const PAGE_SIZE: u32 = 10;
    const MAX_ROUND_SUBMISSIONS: u32 = 256;
    const DEFAULT_MAX_PRICE_DEVIATION_BASIS_POINTS: u32 = 1_000;
    /// Minimum subnet alpha stake required to submit a price (default 10_000_000_000 rao = 10 TAO).
    const DEFAULT_MIN_SUBMITTER_STAKE: u128 = 10_000_000_000;

    /// Snapshot of a committed oracle round, including its final price and source median.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PriceData {
        pub round_id: u32,
        pub price: Ratio,
        pub median_price: Ratio,
        pub reporter_count: u32,
        pub committed_at: u64,
        pub was_overridden: bool,
    }

    /// Metadata attached to a reporter's submission, including the originating hotkey
    /// and an optional provider identifier (e.g. "coinmarketcap", "coingecko").
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PriceSubmissionMetadata {
        pub hot_key: AccountId,
        pub provider: Option<Vec<u8>>,
    }

    /// A single reporter's price submission for an open round.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PriceSubmission {
        pub reporter: AccountId,
        pub price: Ratio,
        pub metadata: PriceSubmissionMetadata,
    }

    /// Lightweight summary of a round used by view callers.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct RoundSummary {
        pub round_id: u32,
        pub reporter_count: u32,
        pub median_price: Option<Ratio>,
    }

    /// Oracle storage: roles, the open round, all submissions, and committed price history.
    #[ink(storage)]
    pub struct TusdtOracle {
        controller: AccountId,
        governance: AccountId,
        /// The Bittensor subnet (netuid) whose registered neurons may submit prices.
        netuid: u16,
        /// Minimum subnet alpha stake required to submit a price.
        min_submitter_stake: u128,

        validator: Option<AccountId>,

        current_round_id: u32,
        round_submissions: Mapping<(u32, AccountId), PriceSubmission>,
        round_reporter_count: Mapping<u32, u32>,
        round_reporters: Mapping<(u32, u32), AccountId>,
        committed_round_prices: Mapping<u32, PriceData>,
        latest_price: Option<PriceData>,
        max_price_deviation: Ratio,
    }

    /// Emitted when a reporter submits (or replaces) a price for the current round.
    #[ink(event)]
    pub struct PriceSubmitted {
        #[ink(topic)]
        round_id: u32,
        #[ink(topic)]
        reporter: AccountId,
        price: Ratio,
        metadata: PriceSubmissionMetadata,
        replaced_existing: bool,
    }

    /// Emitted when a round is committed and a new latest price is recorded.
    #[ink(event)]
    pub struct RoundCommitted {
        #[ink(topic)]
        round_id: u32,
        committed_price: Ratio,
        median_price: Ratio,
        reporter_count: u32,
        was_overridden: bool,
    }

    /// Emitted when oracle governance is transferred to a new account.
    #[ink(event)]
    pub struct OracleGovernanceUpdated {
        #[ink(topic)]
        previous_governance: AccountId,
        #[ink(topic)]
        new_governance: AccountId,
    }

    /// Emitted when the validator account is set or cleared.
    #[ink(event)]
    pub struct ValidatorUpdated {
        #[ink(topic)]
        validator: Option<AccountId>,
    }

    /// Emitted when the maximum allowed price deviation is changed.
    #[ink(event)]
    pub struct MaxPriceDeviationUpdated {
        max_price_deviation: Ratio,
    }

    /// Emitted when the governing subnet netuid is updated.
    #[ink(event)]
    pub struct NetuidUpdated {
        netuid: u16,
    }

    /// Emitted when the minimum required submitter stake is changed.
    #[ink(event)]
    pub struct MinSubmitterStakeUpdated {
        min_submitter_stake: u128,
    }

    /// Emitted when the controller (vault) address is updated by governance.
    #[ink(event)]
    pub struct OracleControllerUpdated {
        #[ink(topic)]
        old_controller: AccountId,
        #[ink(topic)]
        new_controller: AccountId,
    }

    /// Errors returned by the oracle contract.
    #[derive(Debug, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    pub enum Error {
        /// Caller is not the configured controller (vault).
        NotController,
        /// Caller is not the governance account.
        NotGovernance,
        /// Caller is not the configured validator.
        NotValidator,
        /// The supplied hotkey is invalid (e.g. a zero address).
        InvalidHotkey,
        /// The caller's (coldkey, hotkey) pair is not registered in the governing subnet.
        NotRegisteredInSubnet,
        /// The caller's subnet alpha stake is below the required minimum.
        InsufficientStake,
        /// Chain extension call failed at the node level.
        ChainExtensionFailed,
        /// Submitted or overridden price was zero / invalid.
        InvalidPrice,
        /// Round has fewer than `MIN_REPORTERS` submissions for a non-override commit.
        NotEnoughSubmissions,
        /// Round contains no submissions, so a median cannot be computed.
        MedianUnavailable,
        /// The per-round submission cap has been reached.
        MaxSubmissionsReached,
        /// The candidate price moved outside the configured deviation band.
        PriceDeviationExceeded,
        /// Arithmetic overflow or underflow.
        ArithmeticError,
    }

    pub type Result<T> = core::result::Result<T, Error>;

    impl TusdtOracle {
        /// Initializes the oracle contract with controller, governance accounts, and the
        /// Bittensor subnet netuid whose registered neurons are authorized to submit prices.
        #[ink(constructor)]
        pub fn new(controller: AccountId, governance: AccountId, netuid: u16) -> Self {
            Self {
                controller,
                governance,
                netuid,
                min_submitter_stake: DEFAULT_MIN_SUBMITTER_STAKE,
                validator: None,
                current_round_id: 0,
                round_submissions: Mapping::default(),
                round_reporter_count: Mapping::default(),
                round_reporters: Mapping::default(),
                committed_round_prices: Mapping::default(),
                latest_price: None,
                max_price_deviation: Ratio::from_basis_points(
                    DEFAULT_MAX_PRICE_DEVIATION_BASIS_POINTS,
                ),
            }
        }

        /// Submits or replaces the caller's price for the current round. The caller (coldkey)
        /// must supply metadata containing the hotkey of a neuron registered in the governing
        /// subnet. Authorization is verified via the chain extension.
        #[ink(message)]
        pub fn submit_price(
            &mut self,
            price: Ratio,
            metadata: PriceSubmissionMetadata,
        ) -> Result<()> {
            let coldkey = self.env().caller();
            let hotkey = metadata.hot_key;

            // Reject a zero-address hotkey with a clear error.
            let zero_account = AccountId::from([0u8; 32]);
            if hotkey == zero_account {
                return Err(Error::InvalidHotkey);
            }

            // Dynamic authorization via subnet chain extension: the (coldkey, hotkey, netuid)
            // triplet must have a stake record and the hotkey must be registered.
            let info = self
                .env()
                .extension()
                .get_stake_info_for_hotkey_coldkey_netuid(hotkey, coldkey, self.netuid)
                .map_err(|_| Error::ChainExtensionFailed)?
                .ok_or(Error::NotRegisteredInSubnet)?;

            if !info.is_registered {
                return Err(Error::NotRegisteredInSubnet);
            }

            // Verify the caller's subnet alpha stake meets the minimum threshold.
            let stake = info.stake.0 as u128;
            if stake <= self.min_submitter_stake {
                return Err(Error::InsufficientStake);
            }

            if price.is_zero() {
                return Err(Error::InvalidPrice);
            }

            let round_id = self.current_round_id;
            let replaced_existing = self.round_submissions.get((round_id, coldkey)).is_some();
            if !replaced_existing {
                let reporter_count = self.round_reporter_count.get(round_id).unwrap_or(0);
                if reporter_count >= MAX_ROUND_SUBMISSIONS {
                    return Err(Error::MaxSubmissionsReached);
                }
                self.round_reporters
                    .insert((round_id, reporter_count), &coldkey);
                self.round_reporter_count.insert(
                    round_id,
                    &reporter_count
                        .checked_add(1)
                        .ok_or(Error::ArithmeticError)?,
                );
            }

            self.round_submissions.insert(
                (round_id, coldkey),
                &PriceSubmission {
                    reporter: coldkey,
                    price,
                    metadata: metadata.clone(),
                },
            );

            self.env().emit_event(PriceSubmitted {
                round_id,
                reporter: coldkey,
                price,
                metadata,
                replaced_existing,
            });

            Ok(())
        }

        /// Commits the current round using the median submission price or an optional validator override. Validator-only.
        #[ink(message)]
        pub fn commit_round(&mut self, override_price: Option<Ratio>) -> Result<PriceData> {
            self.ensure_validator()?;

            let round_id = self.current_round_id;
            let reporter_count = self.round_reporter_count.get(round_id).unwrap_or(0);
            let round_median = self.compute_round_median(round_id)?;
            let (committed_price, median_price, was_overridden) = match override_price {
                Some(price) if price.is_zero() => return Err(Error::InvalidPrice),
                Some(price) => (price, round_median.unwrap_or(price), true),
                None => {
                    if reporter_count < MIN_REPORTERS {
                        return Err(Error::NotEnoughSubmissions);
                    }
                    let median_price = round_median.ok_or(Error::MedianUnavailable)?;
                    (median_price, median_price, false)
                }
            };
            self.ensure_within_deviation(committed_price)?;
            self.finalize_round(
                round_id,
                committed_price,
                median_price,
                reporter_count,
                was_overridden,
            )
        }

        /// Commits the current round with a governance-supplied price, bypassing quorum and deviation checks. Governance-only.
        #[ink(message)]
        pub fn commit_round_governance(&mut self, price: Ratio) -> Result<PriceData> {
            self.ensure_governance()?;
            if price.is_zero() {
                return Err(Error::InvalidPrice);
            }

            let round_id = self.current_round_id;
            let reporter_count = self.round_reporter_count.get(round_id).unwrap_or(0);
            let median_price = self.compute_round_median(round_id)?.unwrap_or(price);
            self.finalize_round(round_id, price, median_price, reporter_count, true)
        }

        /// Updates the governing subnet netuid. Only governance may call this.
        #[ink(message)]
        pub fn set_netuid(&mut self, netuid: u16) -> Result<()> {
            self.ensure_governance()?;
            self.netuid = netuid;
            self.env().emit_event(NetuidUpdated { netuid });
            Ok(())
        }

        /// Updates the minimum subnet alpha stake required to submit a price. Governance-only.
        #[ink(message)]
        pub fn set_min_submitter_stake(&mut self, min_stake: u128) -> Result<()> {
            self.ensure_governance()?;
            self.min_submitter_stake = min_stake;
            self.env().emit_event(MinSubmitterStakeUpdated {
                min_submitter_stake: min_stake,
            });
            Ok(())
        }

        /// Sets or clears the validator account allowed to commit rounds. Governance-only.
        #[ink(message)]
        pub fn set_validator(&mut self, validator: Option<AccountId>) -> Result<()> {
            self.ensure_governance()?;
            self.validator = validator;
            self.env().emit_event(ValidatorUpdated { validator });
            Ok(())
        }

        /// Updates the maximum allowed deviation between consecutive committed prices. Governance-only.
        #[ink(message)]
        pub fn set_max_price_deviation(&mut self, max_price_deviation: Ratio) -> Result<()> {
            self.ensure_governance()?;
            self.max_price_deviation = max_price_deviation;
            self.env().emit_event(MaxPriceDeviationUpdated {
                max_price_deviation,
            });
            Ok(())
        }

        /// Transfers oracle governance control to a new account. Controller-only.
        #[ink(message)]
        pub fn update_governance(&mut self, new_governance: AccountId) -> Result<()> {
            self.ensure_controller()?;
            let previous_governance = self.governance;
            self.governance = new_governance;
            self.env().emit_event(OracleGovernanceUpdated {
                previous_governance,
                new_governance,
            });
            Ok(())
        }

        /// Transfers the controller (vault) role to a new account. Governance-only.
        /// Used during vault upgrades to hand off control of this oracle to a new
        /// vault instance.
        #[ink(message)]
        pub fn set_controller(&mut self, new_controller: AccountId) -> Result<()> {
            self.ensure_governance()?;
            let old_controller = self.controller;
            self.controller = new_controller;
            self.env().emit_event(OracleControllerUpdated {
                old_controller,
                new_controller,
            });
            Ok(())
        }

        /// Returns the most recently committed oracle price, if any.
        #[ink(message)]
        pub fn get_latest_price(&self) -> Option<PriceData> {
            self.latest_price
        }

        /// Returns the committed price data for a specific round, if it exists.
        #[ink(message)]
        pub fn get_round_price(&self, round_id: u32) -> Option<PriceData> {
            self.committed_round_prices.get(round_id)
        }

        /// Returns the number of committed rounds available in price history.
        #[ink(message)]
        pub fn get_price_history_count(&self) -> u32 {
            self.current_round_id
        }

        /// Returns a paginated history of committed round prices, newest first.
        #[ink(message)]
        pub fn get_price_history(&self, page: u32) -> Vec<PriceData> {
            let Some(latest_round_id) = self.latest_committed_round_id() else {
                return Vec::new();
            };

            let total_prices = self.current_round_id;
            let start = page.saturating_mul(PAGE_SIZE);
            if start >= total_prices {
                return Vec::new();
            }
            let end = min(start.saturating_add(PAGE_SIZE), total_prices);

            let mut history = Vec::new();
            for offset in start..end {
                let round_id = latest_round_id
                    .checked_sub(offset)
                    .expect("round id should exist within computed history page");
                let price_data = self
                    .committed_round_prices
                    .get(round_id)
                    .expect("committed round price should exist");
                history.push(price_data);
            }

            history
        }

        /// Returns all stored submissions for the given round in submission order.
        #[ink(message)]
        pub fn get_round_submissions(&self, round_id: u32) -> Vec<PriceSubmission> {
            let reporter_count = self.round_reporter_count.get(round_id).unwrap_or(0);
            let mut submissions = Vec::with_capacity(reporter_count as usize);

            for index in 0..reporter_count {
                let reporter = self
                    .round_reporters
                    .get((round_id, index))
                    .expect("reporter should exist for round");
                let submission = self
                    .round_submissions
                    .get((round_id, reporter))
                    .expect("submission should exist for reporter");
                submissions.push(submission);
            }

            submissions
        }

        /// Returns the current round summary, including reporter count and median when available.
        #[ink(message)]
        pub fn get_current_round_summary(&self) -> RoundSummary {
            let round_id = self.current_round_id;
            RoundSummary {
                round_id,
                reporter_count: self.round_reporter_count.get(round_id).unwrap_or(0),
                median_price: self.compute_round_median(round_id).unwrap_or(None),
            }
        }

        /// Returns the controller account ID.
        #[ink(message)]
        pub fn controller(&self) -> AccountId {
            self.controller
        }

        /// Returns the governance account ID.
        #[ink(message)]
        pub fn governance(&self) -> AccountId {
            self.governance
        }

        /// Returns the validator account ID, if one is configured.
        #[ink(message)]
        pub fn validator(&self) -> Option<AccountId> {
            self.validator
        }

        /// Returns the current open round ID.
        #[ink(message)]
        pub fn current_round_id(&self) -> u32 {
            self.current_round_id
        }

        /// Returns the maximum number of unique submissions allowed in a round.
        #[ink(message)]
        pub fn max_round_submissions(&self) -> u32 {
            MAX_ROUND_SUBMISSIONS
        }

        /// Returns the maximum allowed deviation between consecutive committed prices.
        #[ink(message)]
        pub fn max_price_deviation(&self) -> Ratio {
            self.max_price_deviation
        }

        /// Returns the governing subnet netuid.
        #[ink(message)]
        pub fn get_netuid(&self) -> u16 {
            self.netuid
        }

        /// Returns the minimum subnet alpha stake required to submit a price.
        #[ink(message)]
        pub fn min_submitter_stake(&self) -> u128 {
            self.min_submitter_stake
        }

        /// Reverts with `NotController` if caller is not the controller (vault) account.
        fn ensure_controller(&self) -> Result<()> {
            if self.env().caller() != self.controller {
                return Err(Error::NotController);
            }
            Ok(())
        }

        /// Reverts with `NotGovernance` if caller is not the governance account.
        fn ensure_governance(&self) -> Result<()> {
            if self.env().caller() != self.governance {
                return Err(Error::NotGovernance);
            }
            Ok(())
        }

        /// Reverts with `NotValidator` if caller is not the currently configured validator.
        fn ensure_validator(&self) -> Result<()> {
            if self.validator != Some(self.env().caller()) {
                return Err(Error::NotValidator);
            }
            Ok(())
        }

        /// Returns the ID of the most recently committed round, or `None` before any commit.
        fn latest_committed_round_id(&self) -> Option<u32> {
            self.current_round_id.checked_sub(1)
        }

        /// Ensures `candidate` is within `max_price_deviation` of the latest committed price.
        fn ensure_within_deviation(&self, candidate: Ratio) -> Result<()> {
            let Some(latest) = self.latest_price else {
                return Ok(());
            };
            if latest.price.is_zero() {
                return Ok(());
            }
            let abs_diff = candidate.abs_diff(latest.price);
            let max_diff = latest
                .price
                .checked_mul(self.max_price_deviation)
                .ok_or(Error::ArithmeticError)?;
            if abs_diff > max_diff {
                return Err(Error::PriceDeviationExceeded);
            }
            Ok(())
        }

        /// Writes the committed price for the round, advances `current_round_id`, and emits `RoundCommitted`.
        fn finalize_round(
            &mut self,
            round_id: u32,
            committed_price: Ratio,
            median_price: Ratio,
            reporter_count: u32,
            was_overridden: bool,
        ) -> Result<PriceData> {
            let price_data = PriceData {
                round_id,
                price: committed_price,
                median_price,
                reporter_count,
                committed_at: self.env().block_timestamp(),
                was_overridden,
            };

            self.committed_round_prices.insert(round_id, &price_data);
            self.latest_price = Some(price_data);
            self.current_round_id = self
                .current_round_id
                .checked_add(1)
                .ok_or(Error::ArithmeticError)?;

            self.env().emit_event(RoundCommitted {
                round_id,
                committed_price,
                median_price,
                reporter_count,
                was_overridden,
            });

            Ok(price_data)
        }

        /// Computes the median of all submitted prices for a round; averages the middle two for even counts.
        fn compute_round_median(&self, round_id: u32) -> Result<Option<Ratio>> {
            let reporter_count = self.round_reporter_count.get(round_id).unwrap_or(0);
            if reporter_count == 0 {
                return Ok(None);
            }

            let mut prices = Vec::with_capacity(reporter_count as usize);
            for index in 0..reporter_count {
                let reporter = self
                    .round_reporters
                    .get((round_id, index))
                    .expect("reporter should exist for round");
                let submission = self
                    .round_submissions
                    .get((round_id, reporter))
                    .expect("submission should exist for reporter");
                prices.push(submission.price);
            }

            prices.sort_unstable();
            let middle_index = prices.len() / 2;
            if prices.len() % 2 == 1 {
                return Ok(prices.get(middle_index).copied());
            }

            let lower = prices
                .get(middle_index.saturating_sub(1))
                .copied()
                .ok_or(Error::MedianUnavailable)?;
            let upper = prices
                .get(middle_index)
                .copied()
                .ok_or(Error::MedianUnavailable)?;
            let average_inner = lower
                .into_inner()
                .checked_add(upper.into_inner())
                .ok_or(Error::ArithmeticError)?
                .checked_div(2)
                .ok_or(Error::ArithmeticError)?;
            Ok(Some(Ratio::from_inner(average_inner)))
        }
    }
}

#[cfg(test)]
mod tests;
