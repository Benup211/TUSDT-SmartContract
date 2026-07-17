#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use self::vault::{
    TusdtVaultAlpha, TusdtVaultAlphaRef, VaultContractParamsConfig, VaultGlobalParamsConfig,
};

#[ink::contract(env = tusdt_env::CustomEnvironment)]
mod vault {
    use core::cmp::min;
    use ink::prelude::vec::Vec;
    use ink::storage::{Mapping, StorageVec};
    use ink::ToAccountId;

    use tusdt_auction::TusdtAuctionRef;
    use tusdt_erc20::TusdtErc20Ref;
    use tusdt_oracle::{PriceData, TusdtOracleRef};
    use tusdt_primitives::Ratio;

    const PAGE_SIZE: u32 = 10;
    pub(crate) const CONTRACT_PARAMS_TIMELOCK_MS: u64 = 24 * 60 * 60 * 1_000;

    mod params {
        include!("params.rs");
    }
    mod interest {
        include!("interest.rs");
    }
    mod risk {
        include!("risk.rs");
    }
    mod vault_access {
        include!("vault_access.rs");
    }

    /// A user's CDP record backed by subnet alpha: alpha stake held, principal borrowed, accrued debt, and timestamps.
    #[derive(Debug, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct Vault {
        pub id: u32,
        pub owner: AccountId,
        /// Which approved subnet this vault's alpha is from.
        pub netuid: u16,
        pub collateral_balance: Balance,
        pub borrowed_token_balance: Balance,
        pub debt_balance: Balance,
        pub total_interest_accrued: Balance,
        pub created_at: u64,
        pub last_interest_accrued_at: u64,
    }

    /// Internal representation of per-netuid risk parameters used by the vault.
    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct VaultContractParams {
        pub collateral_ratio: Ratio,
        pub liquidation_ratio: Ratio,
        pub interest_rate: Ratio,
        pub liquidation_fee: Ratio,
    }

    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    /// External per-netuid config uses basis points for ratio fields, where `100 = 1%`.
    pub struct VaultContractParamsConfig {
        pub collateral_ratio: u32,
        pub liquidation_ratio: u32,
        pub interest_rate: u32,
        pub liquidation_fee: u32,
    }

    /// A queued parameter update awaiting timelock expiry before it can be executed.
    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PendingContractParamsUpdate {
        pub params: VaultContractParamsConfig,
        pub execute_after: u64,
    }

    /// Internal representation of contract-wide (netuid-independent) parameters.
    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct VaultGlobalParams {
        pub transaction_fee: Ratio,
        pub auction_duration_ms: u64,
        pub max_oracle_age_ms: u64,
    }

    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    /// External global config uses basis points for the fee field, where `100 = 1%`.
    pub struct VaultGlobalParamsConfig {
        pub transaction_fee: u32,
        pub auction_duration_ms: u64,
        pub max_oracle_age_ms: u64,
    }

    /// A queued global-parameter update awaiting timelock expiry.
    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PendingGlobalParamsUpdate {
        pub params: VaultGlobalParamsConfig,
        pub execute_after: u64,
    }

    /// Vault storage: roles, child contract refs, risk params, and all per-owner CDP records
    /// backed by subnet alpha stake.
    #[ink(storage)]
    pub struct TusdtVaultAlpha {
        governance: AccountId,
        treasury: AccountId,
        platform: AccountId,
        paused: bool,

        /// Token address of tUSDT.
        token: TusdtErc20Ref,
        /// Auction contract address.
        auction: TusdtAuctionRef,
        /// External oracle providing TUSDT/TAO price.
        oracle: TusdtOracleRef,
        total_collateral_balance: Balance,

        /// The hotkey where all alpha collateral for this vault instance is staked.
        vault_hotkey: AccountId,

        /// Set of netuids whose alpha is accepted as collateral. Managed by governance.
        approved_netuids: Mapping<u16, ()>,

        /// Per-netuid total collateral.  Used to validate that the contract's
        /// actual stake on each subnet covers the sum of all vaults on that subnet.
        netuid_total_collateral: Mapping<u16, Balance>,

        netuid_params: Mapping<u16, VaultContractParams>,
        pending_contract_params_updates: Mapping<u16, PendingContractParamsUpdate>,

        /// Contract-wide parameters shared by all netuids.
        global_params: VaultGlobalParams,
        pending_global_params_update: Option<PendingGlobalParamsUpdate>,

        vaults: Mapping<(AccountId, u32), Vault>,
        owner_total_debt: Mapping<AccountId, Balance>,
        vault_count: Mapping<AccountId, u32>,
        vault_keys: StorageVec<(AccountId, u32)>,
        liquidation_auctions: Mapping<(AccountId, u32), u32>,
        /// Two-step deposit: (caller, netuid) → amount. The caller must register their
        /// intended deposit amount and target subnet before transferring alpha stake
        /// externally, then call `create_alpha_vault` to claim it.
        pending_deposits: Mapping<(AccountId, u16), Balance>,
    }

    /// Emitted when a new alpha-backed vault is opened.
    #[ink(event)]
    pub struct VaultCreated {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        vault_id: u32,
        amount: Balance,
    }

    /// Emitted when alpha collateral is topped up on an existing vault.
    #[ink(event)]
    pub struct CollateralAdded {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        vault_id: u32,
        amount: Balance,
    }

    /// Emitted when alpha collateral is returned to the user.
    #[ink(event)]
    pub struct CollateralReleased {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        vault_id: u32,
        amount: Balance,
        dest_coldkey: AccountId,
    }

    /// Emitted when tUSDT is borrowed against a vault.
    #[ink(event)]
    pub struct TokensBorrowed {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        vault_id: u32,
        amount: Balance,
        transaction_fee: Balance,
    }

    /// Emitted when borrowed tUSDT is repaid.
    #[ink(event)]
    pub struct TokensRepaid {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        vault_id: u32,
        amount: Balance,
        transaction_fee: Balance,
    }

    /// Emitted when interest is accrued onto a vault's debt balance.
    #[ink(event)]
    pub struct InterestAccrued {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        vault_id: u32,
        previous_debt_balance: Balance,
        debt_balance: Balance,
        accrued_at: u64,
    }

    #[ink(event)]
    pub struct ContractParamsUpdated {
        params: VaultContractParamsConfig,
    }

    #[ink(event)]
    pub struct ContractParamsUpdateScheduled {
        params: VaultContractParamsConfig,
        execute_after: u64,
    }

    #[ink(event)]
    pub struct ContractParamsUpdateCancelled {
        params: VaultContractParamsConfig,
    }

    #[ink(event)]
    pub struct GlobalParamsUpdateScheduled {
        params: VaultGlobalParamsConfig,
        execute_after: u64,
    }

    #[ink(event)]
    pub struct GlobalParamsUpdated {
        params: VaultGlobalParamsConfig,
    }

    #[ink(event)]
    pub struct GlobalParamsUpdateCancelled {
        params: VaultGlobalParamsConfig,
    }

    #[ink(event)]
    pub struct VaultGovernanceUpdated {
        #[ink(topic)]
        previous_governance: AccountId,
        #[ink(topic)]
        new_governance: AccountId,
    }

    #[ink(event)]
    pub struct VaultTreasuryUpdated {
        #[ink(topic)]
        previous_treasury: AccountId,
        #[ink(topic)]
        new_treasury: AccountId,
    }

    #[ink(event)]
    pub struct VaultPlatformUpdated {
        #[ink(topic)]
        previous_platform: AccountId,
        #[ink(topic)]
        new_platform: AccountId,
    }

    #[ink(event)]
    pub struct Paused {}

    #[ink(event)]
    pub struct Unpaused {}

    #[ink(event)]
    pub struct SurplusTusdtClaimed {
        #[ink(topic)]
        recipient: AccountId,
        amount: Balance,
    }

    #[ink(event)]
    pub struct ExcessAlphaClaimed {
        #[ink(topic)]
        netuid: u16,
        excess_alpha: Balance,
        tao_received: Balance,
    }

    #[ink(event)]
    pub struct LiquidationAuctionCreated {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        vault_id: u32,
        #[ink(topic)]
        auction_id: u32,
    }

    #[ink(event)]
    pub struct VaultLiquidated {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        vault_id: u32,
        #[ink(topic)]
        auction_id: u32,
        winner: Option<AccountId>,
        winning_bid: Balance,
        collateral_sold: Balance,
        transaction_fee: Balance,
        debt_cleared: Balance,
    }

    /// Errors returned by the alpha vault contract.
    #[derive(Debug, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    pub enum Error {
        VaultNotFound,
        InsufficientCollateral,
        NotVaultOwner,
        TransferFailed,
        InsufficientTokenBalance,
        InvalidTransactionFee,
        TokenBorrowedNotZero,
        InvalidRatio,
        InvalidAuctionDuration,
        CollateralRatioExceeded,
        LiquidationRatioExceeded,
        RepayAmountTooHigh,
        VaultInLiquidation,
        NotLiquidatable,
        LiquidationAuctionExists,
        AuctionContractCallFailed,
        AuctionNotFound,
        AuctionNotFinalized,
        ArithmeticError,
        NotGovernance,
        NotGovernanceOrPlatform,
        ContractPaused,
        OracleCallFailed,
        OraclePriceUnavailable,
        OraclePriceStale,
        InvalidOracleMaxAge,
        NoPendingContractParamsUpdate,
        ContractParamsUpdateTimelockActive,
        /// Chain extension call failed at the node level.
        ChainExtensionFailed,
        /// No alpha stake found for the vault's configured (hotkey, netuid).
        NoAlphaStakeFound,
        /// Caller did not register a deposit intent before calling create_alpha_vault.
        NotDepositor,
        /// The specified netuid is not in the approved set.
        UnapprovedNetuid,
    }

    pub type Result<T> = core::result::Result<T, Error>;

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct DebtPaymentBreakdown {
        pub principal_payment: Balance,
        pub interest_payment: Balance,
    }

    impl TusdtVaultAlpha {
        /// Initializes the alpha vault. `hotkey` is the single staking position for this
        /// vault instance. `oracle_netuid` is the subnet whose registered neurons may
        /// submit oracle prices. Governance must call `set_approved_netuid` to enable
        /// vault creation for specific subnets.
        #[ink(constructor)]
        pub fn new(
            treasury: AccountId,
            token_code_hash: Hash,
            auction_code_hash: Hash,
            oracle_code_hash: Hash,
            oracle_netuid: u16,
            hotkey: AccountId,
        ) -> Self {
            let governance = Self::env().caller();

            let contract_account = Self::env().account_id();
            let token = TusdtErc20Ref::new(contract_account)
                .code_hash(token_code_hash)
                .endowment(0)
                .salt_bytes([0; 32])
                .instantiate();
            let auction = TusdtAuctionRef::new(contract_account, governance, token.to_account_id())
                .code_hash(auction_code_hash)
                .endowment(0)
                .salt_bytes([1; 32])
                .instantiate();
            let oracle = TusdtOracleRef::new(contract_account, governance, oracle_netuid)
                .code_hash(oracle_code_hash)
                .endowment(0)
                .salt_bytes([2; 32])
                .instantiate();

            Self {
                governance,
                treasury,
                platform: governance,
                paused: false,
                token,
                auction,
                oracle,
                total_collateral_balance: 0,
                vault_hotkey: hotkey,
                approved_netuids: Mapping::default(),
                netuid_total_collateral: Mapping::default(),
                netuid_params: Mapping::default(),
                pending_contract_params_updates: Mapping::default(),
                global_params: Self::default_global_params(),
                pending_global_params_update: None,
                vaults: Mapping::default(),
                owner_total_debt: Mapping::default(),
                vault_count: Mapping::default(),
                vault_keys: StorageVec::default(),
                liquidation_auctions: Mapping::default(),
                pending_deposits: Mapping::default(),
            }
        }

        // ── Governance & admin ──────────────────────────────────────────

        /// Schedules a per-netuid parameter update with the standard timelock.
        /// Only governance may call.
        #[ink(message)]
        pub fn set_contract_params(
            &mut self,
            netuid: u16,
            params: VaultContractParamsConfig,
        ) -> Result<()> {
            self.ensure_governance()?;

            Self::contract_params_from_config(params)?;

            let execute_after = self
                .env()
                .block_timestamp()
                .checked_add(CONTRACT_PARAMS_TIMELOCK_MS)
                .ok_or(Error::ArithmeticError)?;
            self.pending_contract_params_updates
                .insert(netuid, &PendingContractParamsUpdate {
                    params,
                    execute_after,
                });

            self.env().emit_event(ContractParamsUpdateScheduled {
                params,
                execute_after,
            });

            Ok(())
        }

        /// Executes the currently scheduled parameter update for a netuid once its timelock has elapsed.
        #[ink(message)]
        pub fn execute_contract_params_update(&mut self, netuid: u16) -> Result<()> {
            let pending = self
                .pending_contract_params_updates
                .get(netuid)
                .ok_or(Error::NoPendingContractParamsUpdate)?;
            if self.env().block_timestamp() < pending.execute_after {
                return Err(Error::ContractParamsUpdateTimelockActive);
            }

            let new_params = Self::contract_params_from_config(pending.params)?;
            self.netuid_params.insert(netuid, &new_params);
            self.pending_contract_params_updates.remove(netuid);

            self.env().emit_event(ContractParamsUpdated {
                params: pending.params,
            });

            Ok(())
        }

        /// Cancels the currently scheduled parameter update for a netuid. Governance only.
        #[ink(message)]
        pub fn cancel_contract_params_update(&mut self, netuid: u16) -> Result<()> {
            self.ensure_governance()?;

            let pending = self
                .pending_contract_params_updates
                .get(netuid)
                .ok_or(Error::NoPendingContractParamsUpdate)?;
            self.pending_contract_params_updates.remove(netuid);

            self.env().emit_event(ContractParamsUpdateCancelled {
                params: pending.params,
            });

            Ok(())
        }

        /// Schedules a global (contract-wide) parameter update with the standard timelock.
        /// Only governance may call.
        #[ink(message)]
        pub fn set_global_params(&mut self, config: VaultGlobalParamsConfig) -> Result<()> {
            self.ensure_governance()?;

            Self::global_params_from_config(config)?;

            let execute_after = self
                .env()
                .block_timestamp()
                .checked_add(CONTRACT_PARAMS_TIMELOCK_MS)
                .ok_or(Error::ArithmeticError)?;
            self.pending_global_params_update = Some(PendingGlobalParamsUpdate {
                params: config,
                execute_after,
            });

            self.env().emit_event(GlobalParamsUpdateScheduled {
                params: config,
                execute_after,
            });

            Ok(())
        }

        /// Executes the currently scheduled global-parameter update once its timelock has elapsed.
        #[ink(message)]
        pub fn execute_global_params_update(&mut self) -> Result<()> {
            let pending = self
                .pending_global_params_update
                .ok_or(Error::NoPendingContractParamsUpdate)?;
            if self.env().block_timestamp() < pending.execute_after {
                return Err(Error::ContractParamsUpdateTimelockActive);
            }

            self.global_params = Self::global_params_from_config(pending.params)?;
            self.pending_global_params_update = None;

            self.env().emit_event(GlobalParamsUpdated {
                params: pending.params,
            });

            Ok(())
        }

        /// Cancels the currently scheduled global-parameter update. Governance only.
        #[ink(message)]
        pub fn cancel_global_params_update(&mut self) -> Result<()> {
            self.ensure_governance()?;

            let pending = self
                .pending_global_params_update
                .take()
                .ok_or(Error::NoPendingContractParamsUpdate)?;

            self.env().emit_event(GlobalParamsUpdateCancelled {
                params: pending.params,
            });

            Ok(())
        }

        #[ink(message)]
        pub fn update_governance(&mut self, new_governance: AccountId) -> Result<()> {
            self.ensure_governance()?;

            self.sync_child_governance(new_governance)?;

            let previous_governance = self.governance;
            self.governance = new_governance;

            self.env().emit_event(VaultGovernanceUpdated {
                previous_governance,
                new_governance,
            });

            Ok(())
        }

        #[ink(message)]
        pub fn update_treasury(&mut self, new_treasury: AccountId) -> Result<()> {
            self.ensure_governance()?;

            let previous_treasury = self.treasury;
            self.treasury = new_treasury;

            self.env().emit_event(VaultTreasuryUpdated {
                previous_treasury,
                new_treasury,
            });

            Ok(())
        }

        #[ink(message)]
        pub fn update_platform(&mut self, new_platform: AccountId) -> Result<()> {
            self.ensure_governance()?;

            let previous_platform = self.platform;
            self.platform = new_platform;

            self.env().emit_event(VaultPlatformUpdated {
                previous_platform,
                new_platform,
            });

            Ok(())
        }

        #[ink(message)]
        pub fn pause(&mut self) -> Result<()> {
            self.ensure_governance_or_platform()?;

            self.paused = true;
            self.env().emit_event(Paused {});

            Ok(())
        }

        #[ink(message)]
        pub fn unpause(&mut self) -> Result<()> {
            self.ensure_governance()?;

            self.paused = false;
            self.env().emit_event(Unpaused {});

            Ok(())
        }

        /// Adds or removes a netuid from the approved set. Only governance may call.
        #[ink(message)]
        pub fn set_approved_netuid(&mut self, netuid: u16, approved: bool) -> Result<()> {
            self.ensure_governance()?;
            if approved {
                self.approved_netuids.insert(netuid, &());
            } else {
                self.approved_netuids.remove(netuid);
            }
            Ok(())
        }

        /// Returns `true` if the given netuid is approved for collateral.
        #[ink(message)]
        pub fn is_approved_netuid(&self, netuid: u16) -> bool {
            self.approved_netuids.get(netuid).is_some()
        }

        /// Transfers surplus TUSDT held by the vault contract to the treasury.
        /// Callable by governance or platform.
        #[ink(message)]
        pub fn claim_surplus_tusdt(&mut self, amount: Balance) -> Result<()> {
            self.ensure_governance_or_platform()?;
            self.token
                .transfer(self.treasury, amount)
                .map_err(|_| Error::TransferFailed)?;

            self.env().emit_event(SurplusTusdtClaimed {
                recipient: self.treasury,
                amount,
            });

            Ok(())
        }

        /// Claims excess alpha staking rewards on a subnet by unstaking them to native
        /// TAO and transferring the TAO to the treasury. Governance only.
        ///
        /// Excess = actual contract stake on the netuid minus total vault collateral on
        /// that netuid (`netuid_total_collateral`).  If there is no excess, this is a
        /// no-op (returns Ok).
        #[ink(message)]
        pub fn claim_excess_alpha(&mut self, netuid: u16) -> Result<()> {
            self.ensure_governance()?;

            // Read actual available stake.  If none exists on this netuid, nothing to claim.
            let current_stake = match self.get_contract_stake(netuid) {
                Ok(s) => s,
                Err(Error::NoAlphaStakeFound) => return Ok(()),
                Err(e) => return Err(e),
            };

            let netuid_total = self.netuid_total_collateral.get(netuid).unwrap_or_default();

            if current_stake <= netuid_total {
                return Ok(());
            }

            let excess = current_stake
                .checked_sub(netuid_total)
                .ok_or(Error::ArithmeticError)?;

            // Snapshot native TAO balance before unstaking.
            let balance_before = self.env().balance();

            // Unstake excess alpha to native TAO via chain extension.
            self.env()
                .extension()
                .remove_stake(self.vault_hotkey, netuid, excess)
                .map_err(|_| Error::ChainExtensionFailed)?;

            // Read the actual TAO received from the balance change.
            let balance_after = self.env().balance();
            let tao_received = balance_after
                .checked_sub(balance_before)
                .ok_or(Error::ArithmeticError)?;

            // Transfer TAO to treasury.
            if tao_received > 0 {
                self.env()
                    .transfer(self.treasury, tao_received)
                    .map_err(|_| Error::TransferFailed)?;
            }

            self.env().emit_event(ExcessAlphaClaimed {
                netuid,
                excess_alpha: excess,
                tao_received,
            });

            Ok(())
        }

        // ── Alpha vault lifecycle ───────────────────────────────────────

        /// Step 1 of the two-step alpha deposit: registers the caller's intent to deposit
        /// the given amount of alpha stake on the specified subnet. After calling this, the
        /// user must transfer their alpha stake for this vault's hotkey to this contract via
        /// a subtensor `transfer_stake` extrinsic, then call `create_alpha_vault(netuid)`.
        #[ink(message)]
        pub fn deposit_alpha(&mut self, amount: Balance, netuid: u16) -> Result<()> {
            self.ensure_not_paused()?;
            self.ensure_approved_netuid(netuid)?;

            if amount == 0 {
                return Err(Error::InsufficientCollateral);
            }
            let caller = self.env().caller();
            if self.pending_deposits.get((caller, netuid)).is_some() {
                return Err(Error::InsufficientCollateral);
            }

            self.pending_deposits.insert((caller, netuid), &amount);
            Ok(())
        }

        /// Creates a new alpha-backed vault for the caller on the specified subnet. The
        /// caller must have first called `deposit_alpha(amount, netuid)` to register intent,
        /// then transferred alpha stake for this vault's hotkey to this contract via a
        /// subtensor `transfer_stake` extrinsic.
        #[ink(message)]
        pub fn create_alpha_vault(&mut self, netuid: u16) -> Result<u32> {
            self.ensure_not_paused()?;
            self.ensure_approved_netuid(netuid)?;

            let caller = self.env().caller();

            let amount = self
                .pending_deposits
                .get((caller, netuid))
                .ok_or(Error::NotDepositor)?;

            // Verify the contract holds enough stake on this subnet to cover all
            // existing vaults on this netuid plus this new allocation.
            let current_stake = self.get_contract_stake(netuid)?;
            let netuid_total = self.netuid_total_collateral.get(netuid).unwrap_or_default();
            let required = netuid_total
                .checked_add(amount)
                .ok_or(Error::ArithmeticError)?;
            if current_stake < required {
                return Err(Error::InsufficientCollateral);
            }

            let (_, projected_total) = self.ensure_collateral_bounds(netuid, 0, amount)?;

            let timestamp = self.env().block_timestamp();

            let vault_id = self.vault_count.get(caller).unwrap_or(0);
            let vault = Vault {
                id: vault_id,
                owner: caller,
                netuid,
                collateral_balance: amount,
                borrowed_token_balance: 0,
                debt_balance: 0,
                total_interest_accrued: 0,
                created_at: timestamp,
                last_interest_accrued_at: timestamp,
            };

            self.save_vault(caller, vault_id, &vault)?;
            self.vault_keys.push(&(caller, vault_id));
            self.pending_deposits.remove((caller, netuid));
            self.total_collateral_balance = projected_total;
            self.netuid_total_collateral.insert(netuid, &required);

            let next_id = vault_id.checked_add(1).ok_or(Error::ArithmeticError)?;
            self.vault_count.insert(caller, &next_id);

            self.env().emit_event(VaultCreated {
                owner: caller,
                vault_id,
                amount,
            });

            Ok(vault_id)
        }

        /// Re-syncs an alpha vault's collateral from the chain. The caller must have
        /// transferred additional alpha stake to this contract externally. This message
        /// queries the chain extension to discover the new stake amount and updates the
        /// vault record.
        #[ink(message)]
        pub fn add_alpha_collateral(&mut self, vault_id: u32) -> Result<()> {
            self.ensure_not_paused()?;

            let (caller, mut vault) = self.load_caller_vault(vault_id)?;
            let current_stake = self.get_contract_stake(vault.netuid)?;

            let netuid_total = self
                .netuid_total_collateral
                .get(vault.netuid)
                .unwrap_or_default();
            let addition = current_stake
                .checked_sub(netuid_total)
                .ok_or(Error::InsufficientCollateral)?;

            let (projected_vault, projected_total) =
                self.ensure_collateral_bounds(vault.netuid, vault.collateral_balance, addition)?;

            vault.collateral_balance = projected_vault;
            self.total_collateral_balance = projected_total;
            let projected_netuid = netuid_total
                .checked_add(addition)
                .ok_or(Error::ArithmeticError)?;
            self.netuid_total_collateral
                .insert(vault.netuid, &projected_netuid);
            self.save_vault(caller, vault_id, &vault)?;

            self.env().emit_event(CollateralAdded {
                owner: caller,
                vault_id,
                amount: addition,
            });

            Ok(())
        }

        /// Borrows tokens against the vault's alpha collateral.
        #[ink(message)]
        pub fn borrow_token(&mut self, vault_id: u32, amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;

            let (caller, mut vault) = self.load_caller_vault(vault_id)?;

            self.accrue_interest_for_vault(&mut vault)?;

            if amount.eq(&0) {
                self.save_vault(caller, vault_id, &vault)?;
                return Ok(());
            }
            let fee = self.calculate_transaction_fee(amount)?;
            let net_borrow_amount = amount.checked_sub(fee).ok_or(Error::ArithmeticError)?;

            let price = self.current_collateral_price(vault.netuid)?;

            let max_borrow = self.max_borrow_allowed(vault.netuid, price, vault.collateral_balance)?;
            let projected_borrowed = vault
                .borrowed_token_balance
                .checked_add(amount)
                .ok_or(Error::ArithmeticError)?;
            let projected_debt = vault
                .debt_balance
                .checked_add(amount)
                .ok_or(Error::ArithmeticError)?;
            if projected_debt > max_borrow {
                return Err(Error::CollateralRatioExceeded);
            }

            if net_borrow_amount > 0 {
                self.token
                    .mint(caller, net_borrow_amount)
                    .map_err(|_| Error::TransferFailed)?;
            }
            if fee > 0 {
                self.token
                    .mint(self.treasury, fee)
                    .map_err(|_| Error::TransferFailed)?;
            }

            self.adjust_last_interest_accrued_at_for_new_borrow(&mut vault, amount)?;
            vault.borrowed_token_balance = projected_borrowed;
            vault.debt_balance = projected_debt;
            self.save_vault(caller, vault_id, &vault)?;

            self.env().emit_event(TokensBorrowed {
                owner: caller,
                vault_id,
                amount,
                transaction_fee: fee,
            });

            Ok(())
        }

        /// Repays borrowed tokens from an alpha vault.
        #[ink(message)]
        pub fn repay_token(&mut self, vault_id: u32, amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;

            let (caller, mut vault) = self.load_caller_vault(vault_id)?;

            self.accrue_interest_for_vault(&mut vault)?;
            if amount.eq(&0) {
                self.save_vault(caller, vault_id, &vault)?;
                return Ok(());
            }
            if amount > vault.debt_balance {
                return Err(Error::RepayAmountTooHigh);
            }
            let fee = self.calculate_transaction_fee(amount)?;
            let total_token_charge = amount.checked_add(fee).ok_or(Error::ArithmeticError)?;
            self.ensure_token_balance_at_least(caller, total_token_charge)?;

            let payment = Self::apply_debt_payment(&mut vault, amount)?;

            self.token
                .burn(caller, total_token_charge)
                .map_err(|_| Error::TransferFailed)?;
            let treasury_mint = payment
                .interest_payment
                .checked_add(fee)
                .ok_or(Error::ArithmeticError)?;
            if treasury_mint > 0 {
                self.token
                    .mint(self.treasury, treasury_mint)
                    .map_err(|_| Error::TransferFailed)?;
            }

            self.save_vault(caller, vault_id, &vault)?;

            self.env().emit_event(TokensRepaid {
                owner: caller,
                vault_id,
                amount,
                transaction_fee: fee,
            });

            Ok(())
        }

        /// Accrues any elapsed interest for a vault.
        #[ink(message)]
        pub fn accrue_interest(&mut self, owner: AccountId, vault_id: u32) -> Result<Balance> {
            self.ensure_not_paused()?;

            self.ensure_not_in_liquidation(owner, vault_id)?;

            let mut vault = self.load_vault(owner, vault_id)?;
            let previous_debt_balance = vault.debt_balance;

            self.accrue_interest_for_vault(&mut vault)?;
            let debt_balance = vault.debt_balance;
            let accrued_at = vault.last_interest_accrued_at;

            self.save_vault(owner, vault_id, &vault)?;
            self.env().emit_event(InterestAccrued {
                owner,
                vault_id,
                previous_debt_balance,
                debt_balance,
                accrued_at,
            });

            Ok(debt_balance)
        }

        /// Releases alpha collateral from a vault, returning the stake to a destination
        /// coldkey via the chain extension. Validates that the remaining collateral still
        /// satisfies ratio requirements.
        #[ink(message)]
        pub fn release_alpha_collateral(
            &mut self,
            vault_id: u32,
            amount: Balance,
            dest_coldkey: AccountId,
        ) -> Result<()> {
            self.ensure_not_paused()?;

            let (caller, mut vault) = self.load_caller_vault(vault_id)?;

            if vault.collateral_balance < amount {
                return Err(Error::InsufficientCollateral);
            }

            self.accrue_interest_for_vault(&mut vault)?;
            let projected_collateral = vault
                .collateral_balance
                .checked_sub(amount)
                .ok_or(Error::ArithmeticError)?;
            if vault.debt_balance > 0 {
                let price = self.current_collateral_price(vault.netuid)?;
                let max_borrow_after_release =
                    self.max_borrow_allowed(vault.netuid, price, projected_collateral)?;
                if vault.debt_balance > max_borrow_after_release {
                    return Err(Error::CollateralRatioExceeded);
                }
            }

            // Transfer stake back to the user via chain extension.
            self.env()
                .extension()
                .transfer_stake(dest_coldkey, self.vault_hotkey, vault.netuid, vault.netuid, amount)
                .map_err(|_| Error::ChainExtensionFailed)?;

            self.total_collateral_balance = self
                .total_collateral_balance
                .checked_sub(amount)
                .ok_or(Error::ArithmeticError)?;
            let netuid_total = self
                .netuid_total_collateral
                .get(vault.netuid)
                .unwrap_or_default();
            self.netuid_total_collateral.insert(
                vault.netuid,
                &netuid_total.checked_sub(amount).ok_or(Error::ArithmeticError)?,
            );
            vault.collateral_balance = projected_collateral;
            self.save_vault(caller, vault_id, &vault)?;

            self.env().emit_event(CollateralReleased {
                owner: caller,
                vault_id,
                amount,
                dest_coldkey,
            });

            Ok(())
        }

        /// Initiates a liquidation auction for an unsafe alpha vault. This first unstakes
        /// the alpha collateral to native TAO via the chain extension, then creates a
        /// standard Dutch auction with the recovered TAO.
        #[ink(message)]
        pub fn trigger_liquidation_auction(
            &mut self,
            owner: AccountId,
            vault_id: u32,
        ) -> Result<u32> {
            self.ensure_not_paused()?;

            if self.liquidation_auctions.get((owner, vault_id)).is_some() {
                return Err(Error::LiquidationAuctionExists);
            }

            let mut vault = self.load_vault(owner, vault_id)?;
            self.accrue_interest_for_vault(&mut vault)?;
            let price = self.current_collateral_price(vault.netuid)?;

            if !self.is_liquidatable(price, &vault)? {
                return Err(Error::NotLiquidatable);
            }

            // Unstake all alpha collateral to recover TAO into the contract's balance.
            let collateral_amount = vault.collateral_balance;
            self.env()
                .extension()
                .remove_stake(self.vault_hotkey, vault.netuid, collateral_amount)
                .map_err(|_| Error::TransferFailed)?;

            // Compute how much TAO was actually received from unstaking alpha.
            // The chain extension's alpha price is RAO per alpha, scaled by 1e9.
            let alpha_price_rao = self
                .env()
                .extension()
                .get_alpha_price(vault.netuid)
                .map_err(|_| Error::ChainExtensionFailed)?;
            let alpha_to_tao = Self::alpha_price_rao_to_ratio(alpha_price_rao)?;
            let tao_received = alpha_to_tao
                .checked_mul_value(u128::from(collateral_amount))
                .ok_or(Error::ArithmeticError)?;
            let tao_received =
                Balance::try_from(tao_received).map_err(|_| Error::ArithmeticError)?;

            // Zero out the vault's collateral immediately — the alpha has been
            // unstaked and the TAO now sits in the contract's balance.
            vault.collateral_balance = 0;
            self.total_collateral_balance = self
                .total_collateral_balance
                .checked_sub(collateral_amount)
                .ok_or(Error::ArithmeticError)?;
            let netuid_total = self
                .netuid_total_collateral
                .get(vault.netuid)
                .unwrap_or_default();
            self.netuid_total_collateral.insert(
                vault.netuid,
                &netuid_total.checked_sub(collateral_amount)
                    .ok_or(Error::ArithmeticError)?,
            );
            self.save_vault(owner, vault_id, &vault)?;

            // Now the TAO is in the contract's balance. Create a standard auction
            // with the actual TAO amount received.
            let min_bid = self.liquidation_min_bid(vault.netuid, vault.debt_balance)?;
            let auction_id = self
                .auction
                .create_auction(
                    owner,
                    vault_id,
                    tao_received,
                    vault.debt_balance,
                    min_bid,
                    price,
                    Some(self.global_params.auction_duration_ms),
                )
                .map_err(|_| Error::AuctionContractCallFailed)?;

            self.liquidation_auctions
                .insert((owner, vault_id), &auction_id);

            self.env().emit_event(LiquidationAuctionCreated {
                owner,
                vault_id,
                auction_id,
            });

            Ok(auction_id)
        }

        /// Settles a finalized liquidation auction, transferring collateral to the winner,
        /// routing accrued interest to the treasury, and burning only principal.
        #[ink(message)]
        pub fn settle_liquidation_auction(
            &mut self,
            owner: AccountId,
            vault_id: u32,
        ) -> Result<()> {
            let auction_id = self
                .liquidation_auctions
                .get((owner, vault_id))
                .ok_or(Error::AuctionNotFound)?;

            let auction = self
                .auction
                .get_auction(auction_id)
                .ok_or(Error::AuctionNotFound)?;

            if !auction.is_finalized || auction.highest_bidder.is_none() {
                return Err(Error::AuctionNotFinalized);
            }

            let winner = auction
                .highest_bidder
                .expect("checked winner presence above");
            let winning_bid = auction.highest_bid;

            let mut vault = self.load_vault(owner, vault_id)?;
            let collateral_sold = auction.collateral_balance;
            let transaction_fee = self.calculate_transaction_fee(collateral_sold)?;
            let debt_cleared = auction.debt_balance;
            let payment = Self::apply_debt_payment(&mut vault, debt_cleared)?;

            // ── Effects first (CEI): collateral was already zeroed and subtracted
            //     from total_collateral_balance in trigger_liquidation_auction when
            //     the alpha was unstaked.  Only vault debt state and the liquidation
            //     marker need cleanup here.
            self.save_vault(owner, vault_id, &vault)?;
            self.liquidation_auctions.remove((owner, vault_id));

            // ── Interactions: external calls and transfers.
            self.auction
                .transfer_winning_bid(auction_id, self.env().account_id())
                .map_err(|_| Error::AuctionContractCallFailed)?;

            let winner_collateral = collateral_sold
                .checked_sub(transaction_fee)
                .ok_or(Error::ArithmeticError)?;

            if transaction_fee > 0 {
                self.transfer_transaction_fee_to_treasury(transaction_fee)?;
            }
            if winner_collateral > 0 && self.env().transfer(winner, winner_collateral).is_err() {
                return Err(Error::TransferFailed);
            }
            self.token
                .burn(self.env().account_id(), payment.principal_payment)
                .map_err(|_| Error::TransferFailed)?;
            if payment.interest_payment > 0 {
                self.token
                    .transfer(self.treasury, payment.interest_payment)
                    .map_err(|_| Error::TransferFailed)?;
            }

            self.env().emit_event(VaultLiquidated {
                owner,
                vault_id,
                auction_id,
                winner: Some(winner),
                winning_bid,
                collateral_sold,
                transaction_fee,
                debt_cleared,
            });

            Ok(())
        }

        // ── Read methods ────────────────────────────────────────────────

        #[ink(message)]
        pub fn get_vault(&self, owner: AccountId, vault_id: u32) -> Option<Vault> {
            self.vaults.get((owner, vault_id))
        }

        #[ink(message)]
        pub fn get_token_address(&self) -> AccountId {
            self.token.to_account_id()
        }

        #[ink(message)]
        pub fn get_auction_address(&self) -> AccountId {
            self.auction.to_account_id()
        }

        #[ink(message)]
        pub fn get_oracle_address(&self) -> AccountId {
            self.oracle.to_account_id()
        }

        #[ink(message)]
        pub fn governance(&self) -> AccountId {
            self.governance
        }

        #[ink(message)]
        pub fn treasury(&self) -> AccountId {
            self.treasury
        }

        #[ink(message)]
        pub fn platform(&self) -> AccountId {
            self.platform
        }

        #[ink(message)]
        pub fn paused(&self) -> bool {
            self.paused
        }

        #[ink(message)]
        pub fn get_contract_params(&self, netuid: u16) -> VaultContractParamsConfig {
            Self::contract_params_to_config(self.get_params(netuid))
        }

        #[ink(message)]
        pub fn get_global_params(&self) -> VaultGlobalParamsConfig {
            Self::global_params_to_config(self.global_params)
        }

        #[ink(message)]
        pub fn get_pending_contract_params_update(
            &self,
            netuid: u16,
        ) -> Option<PendingContractParamsUpdate> {
            self.pending_contract_params_updates.get(netuid)
        }

        #[ink(message)]
        pub fn get_vault_collateral_balance(
            &self,
            owner: AccountId,
            vault_id: u32,
        ) -> Option<Balance> {
            self.vaults
                .get((owner, vault_id))
                .map(|v| v.collateral_balance)
        }

        #[ink(message)]
        pub fn get_total_collateral_balance(&self) -> Balance {
            self.total_collateral_balance
        }

        #[ink(message)]
        pub fn get_total_debt(&self, owner: AccountId) -> Balance {
            self.owner_total_debt.get(owner).unwrap_or_default()
        }

        #[ink(message)]
        pub fn get_vault_collateral_value(
            &self,
            owner: AccountId,
            vault_id: u32,
        ) -> Result<Balance> {
            let vault = self
                .vaults
                .get((owner, vault_id))
                .ok_or(Error::VaultNotFound)?;
            let price = self.current_collateral_price(vault.netuid)?;
            Self::collateral_value(price, vault.collateral_balance)
        }

        #[ink(message)]
        pub fn get_max_borrow(&self, owner: AccountId, vault_id: u32) -> Result<Balance> {
            let vault = self
                .vaults
                .get((owner, vault_id))
                .ok_or(Error::VaultNotFound)?;
            let price = self.current_collateral_price(vault.netuid)?;
            let max = self.max_borrow_allowed(vault.netuid, price, vault.collateral_balance)?;

            Ok(max)
        }

        #[ink(message)]
        pub fn get_liquidation_auction_id(&self, owner: AccountId, vault_id: u32) -> Option<u32> {
            self.liquidation_auctions.get((owner, vault_id))
        }

        #[ink(message)]
        pub fn get_total_vaults_count(&self) -> u32 {
            self.vault_keys.len()
        }

        #[ink(message)]
        pub fn get_vaults_count(&self, owner: AccountId) -> u32 {
            self.vault_count.get(owner).unwrap_or_default()
        }

        #[ink(message)]
        pub fn get_vaults(&self, owner: AccountId, page: u32) -> Result<Vec<Vault>> {
            let total_owner_vaults = self.vault_count.get(owner).unwrap_or_default();
            let start = page.saturating_mul(PAGE_SIZE);
            if start >= total_owner_vaults {
                return Ok(Vec::new());
            }
            let end = min(start.saturating_add(PAGE_SIZE), total_owner_vaults);

            let mut vaults = Vec::new();
            for index in start..end {
                let vault = self.vaults.get((owner, index));
                vaults.push(vault.expect("should be present"));
            }

            Ok(vaults)
        }

        #[ink(message)]
        pub fn get_all_vaults(&self, page: u32) -> Result<Vec<Vault>> {
            let total_vaults = self.vault_keys.len();
            let start = page.saturating_mul(PAGE_SIZE);
            if start >= total_vaults {
                return Ok(Vec::new());
            }
            let end = min(start.saturating_add(PAGE_SIZE), total_vaults);

            let mut vaults = Vec::new();
            for index in start..end {
                let key = self.vault_keys.get(index).expect("should be present");
                let vault = self.vaults.get(key);
                vaults.push(vault.expect("should be present"));
            }

            Ok(vaults)
        }

        // ── Internal helpers ────────────────────────────────────────────

        /// Queries the chain extension for the alpha stake held by this contract as coldkey
        /// for the vault's configured hotkey on the given subnet. Returns only the
        /// available (unlocked) portion.
        fn get_contract_stake(&self, netuid: u16) -> Result<Balance> {
            let contract_addr = self.env().account_id();
            let info = self
                .env()
                .extension()
                .get_stake_info_for_hotkey_coldkey_netuid(
                    self.vault_hotkey,
                    contract_addr,
                    netuid,
                )
                .map_err(|_| Error::ChainExtensionFailed)?
                .ok_or(Error::NoAlphaStakeFound)?;

            let stake: u64 = info.stake.into();
            let locked: u64 = info.locked.into();
            let available = stake.checked_sub(locked).ok_or(Error::ArithmeticError)?;

            if available == 0 {
                return Err(Error::NoAlphaStakeFound);
            }
            Ok(available)
        }

        /// Returns the active params for a netuid, falling back to defaults when none configured.
        fn get_params(&self, netuid: u16) -> VaultContractParams {
            self.netuid_params
                .get(netuid)
                .unwrap_or_else(Self::default_contract_params)
        }

        /// Returns the collateral price for a given subnet: TUSDT per alpha unit.
        ///
        /// Combines two sources:
        /// 1. TUSDT/TAO from the oracle contract
        /// 2. Alpha/TAO from the chain extension (`get_alpha_price`, scaled by 1e9)
        ///
        /// Formula: `tusdt_per_alpha = tusdt_per_tao × (alpha_price_rao / 1_000_000_000)`
        pub(crate) fn current_collateral_price(&self, netuid: u16) -> Result<Ratio> {
            // TUSDT per TAO from oracle
            let price_data = Self::validate_price_data(
                self.oracle.get_latest_price(),
                self.env().block_timestamp(),
                self.global_params.max_oracle_age_ms,
            )?;
            let tusdt_per_tao = price_data.price;

            // Alpha per TAO from chain extension (RAO per alpha, 1 TAO = 1e9 RAO)
            let alpha_price_rao = self
                .env()
                .extension()
                .get_alpha_price(netuid)
                .map_err(|_| Error::ChainExtensionFailed)?;

            // Convert: alpha_to_tao = alpha_price_rao / 1_000_000_000
            let alpha_to_tao = Self::alpha_price_rao_to_ratio(alpha_price_rao)?;

            // TUSDT per alpha = TUSDT per TAO × (alpha per TAO)
            tusdt_per_tao
                .checked_mul(alpha_to_tao)
                .ok_or(Error::ArithmeticError)
        }

        pub(crate) fn validate_price_data(
            price_data: Option<PriceData>,
            now: u64,
            max_oracle_age_ms: u64,
        ) -> Result<PriceData> {
            let price_data = price_data.ok_or(Error::OraclePriceUnavailable)?;
            let age = now
                .checked_sub(price_data.committed_at)
                .ok_or(Error::OraclePriceStale)?;
            if age > max_oracle_age_ms {
                return Err(Error::OraclePriceStale);
            }
            if price_data.price.is_zero() {
                return Err(Error::OraclePriceUnavailable);
            }
            Ok(price_data)
        }

        pub(crate) fn sync_owner_total_debt(
            &mut self,
            owner: AccountId,
            previous_vault_debt: Balance,
            next_vault_debt: Balance,
        ) -> Result<()> {
            let owner_total_debt = self.owner_total_debt.get(owner).unwrap_or_default();
            let owner_total_debt = owner_total_debt
                .checked_sub(previous_vault_debt)
                .ok_or(Error::ArithmeticError)?
                .checked_add(next_vault_debt)
                .ok_or(Error::ArithmeticError)?;
            self.owner_total_debt.insert(owner, &owner_total_debt);
            Ok(())
        }

        pub(crate) fn apply_debt_payment(
            vault: &mut Vault,
            payment_amount: Balance,
        ) -> Result<DebtPaymentBreakdown> {
            if payment_amount > vault.debt_balance {
                return Err(Error::RepayAmountTooHigh);
            }

            let outstanding_interest = Self::outstanding_interest(vault)?;
            let interest_payment = core::cmp::min(payment_amount, outstanding_interest);
            let principal_payment = payment_amount
                .checked_sub(interest_payment)
                .ok_or(Error::ArithmeticError)?;

            vault.debt_balance = vault
                .debt_balance
                .checked_sub(payment_amount)
                .ok_or(Error::ArithmeticError)?;
            vault.borrowed_token_balance = vault
                .borrowed_token_balance
                .checked_sub(principal_payment)
                .ok_or(Error::ArithmeticError)?;

            Ok(DebtPaymentBreakdown {
                principal_payment,
                interest_payment,
            })
        }

        pub(crate) fn outstanding_interest(vault: &Vault) -> Result<Balance> {
            vault
                .debt_balance
                .checked_sub(vault.borrowed_token_balance)
                .ok_or(Error::ArithmeticError)
        }

        pub(crate) fn calculate_transaction_fee(&self, amount: Balance) -> Result<Balance> {
            self.global_params
                .transaction_fee
                .checked_mul_value(amount.into())
                .and_then(|fee| Balance::try_from(fee).ok())
                .ok_or(Error::ArithmeticError)
        }

        /// Converts a raw alpha price from the chain extension (function 15, RAO per
        /// alpha — TAO/alpha scaled by 1e9) into a `Ratio` of TAO per alpha.
        /// E.g. `377_277` → `0.000377277`, `1_000_000_000` → `1.0`.
        #[inline]
        pub(crate) fn alpha_price_rao_to_ratio(alpha_price_rao: u64) -> Result<Ratio> {
            Ratio::from_integer(u128::from(alpha_price_rao))
                .checked_div_int(1_000_000_000u128)
                .ok_or(Error::ArithmeticError)
        }

        #[inline]
        pub(crate) fn ensure_token_balance_at_least(
            &self,
            owner: AccountId,
            required_balance: Balance,
        ) -> Result<()> {
            if self.token.balance_of(owner) < required_balance {
                return Err(Error::InsufficientTokenBalance);
            }
            Ok(())
        }

        #[inline]
        pub(crate) fn transfer_transaction_fee_to_treasury(&mut self, fee: Balance) -> Result<()> {
            if fee == 0 {
                return Ok(());
            }
            if self.env().transfer(self.treasury, fee).is_err() {
                return Err(Error::TransferFailed);
            }
            Ok(())
        }

        #[inline]
        fn ensure_governance(&self) -> Result<()> {
            if self.env().caller() != self.governance {
                return Err(Error::NotGovernance);
            }
            Ok(())
        }

        #[inline]
        fn ensure_governance_or_platform(&self) -> Result<()> {
            let caller = self.env().caller();
            if caller != self.governance && caller != self.platform {
                return Err(Error::NotGovernanceOrPlatform);
            }
            Ok(())
        }

        #[inline]
        fn ensure_not_paused(&self) -> Result<()> {
            if self.paused {
                return Err(Error::ContractPaused);
            }
            Ok(())
        }

        #[inline]
        fn ensure_approved_netuid(&self, netuid: u16) -> Result<()> {
            if self.approved_netuids.get(netuid).is_none() {
                return Err(Error::UnapprovedNetuid);
            }
            Ok(())
        }

        #[cfg(not(test))]
        fn sync_child_governance(&mut self, new_governance: AccountId) -> Result<()> {
            self.auction
                .update_governance(new_governance)
                .map_err(|_| Error::AuctionContractCallFailed)?;
            self.oracle
                .update_governance(new_governance)
                .map_err(|_| Error::OracleCallFailed)?;
            Ok(())
        }

        #[cfg(test)]
        fn sync_child_governance(&mut self, _new_governance: AccountId) -> Result<()> {
            Ok(())
        }
    }

    #[cfg(test)]
    impl TusdtVaultAlpha {
        pub(crate) fn new_for_test(governance: AccountId) -> Self {
            use ink::env::call::FromAccountId;

            let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();

            Self {
                governance,
                treasury: governance,
                platform: governance,
                paused: false,
                token: TusdtErc20Ref::from_account_id(accounts.charlie),
                auction: TusdtAuctionRef::from_account_id(accounts.django),
                oracle: TusdtOracleRef::from_account_id(accounts.eve),
                total_collateral_balance: 0,
                vault_hotkey: accounts.bob,
                approved_netuids: Mapping::default(),
                netuid_total_collateral: Mapping::default(),
                netuid_params: Mapping::default(),
                pending_contract_params_updates: Mapping::default(),
                global_params: Self::default_global_params(),
                pending_global_params_update: None,
                vaults: Mapping::default(),
                owner_total_debt: Mapping::default(),
                vault_count: Mapping::default(),
                vault_keys: StorageVec::default(),
                liquidation_auctions: Mapping::default(),
                pending_deposits: Mapping::default(),
            }
        }

        pub(crate) fn set_liquidation_auction_for_test(
            &mut self,
            owner: AccountId,
            vault_id: u32,
            auction_id: u32,
        ) {
            self.liquidation_auctions
                .insert((owner, vault_id), &auction_id);
        }
    }
}

#[cfg(test)]
mod tests;
