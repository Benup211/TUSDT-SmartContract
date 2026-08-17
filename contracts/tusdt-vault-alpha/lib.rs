#![cfg_attr(not(feature = "std"), no_std, no_main)]
#![allow(clippy::enum_variant_names)]

pub use self::vault::{
    TusdtVaultAlpha, TusdtVaultAlphaRef, VaultContractParamsConfig, VaultGlobalParamsConfig,
};

#[ink::contract(env = tusdt_env::CustomEnvironment)]
mod vault {
    use core::cmp::min;
    use ink::prelude::vec::Vec;
    use ink::storage::{Mapping, StorageVec};
    use ink::{env::call::FromAccountId, ToAccountId};

    use tusdt_auction::TusdtAuctionRef;
    use tusdt_erc20::TusdtErc20Ref;
    use tusdt_oracle::{PriceData, TusdtOracleRef};
    use tusdt_primitives::Ratio;

    const PAGE_SIZE: u32 = 10;
    /// Standard timelock (24 hours) that scheduled contract and global parameter updates
    /// must wait before they can be executed.
    pub(crate) const CONTRACT_PARAMS_TIMELOCK_MS: u64 = 60 * 1_000;

    mod params {
        include!("params.rs");
    }
    mod risk {
        include!("risk.rs");
    }
    mod vault_access {
        include!("vault_access.rs");
    }

    /// A user's CDP record backed by subnet alpha: alpha stake held and principal borrowed.
    #[derive(Debug, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct Vault {
        /// Per-owner sequential vault identifier.
        pub id: u32,
        /// Account that opened the vault and controls its collateral.
        pub owner: AccountId,
        /// Which approved subnet this vault's alpha is from.
        pub netuid: u16,
        /// Alpha stake held as collateral (Balance = u64, 9 decimals).
        pub collateral_balance: Balance,
        /// Outstanding tUSDT borrowed against this vault.
        pub borrowed_token_balance: Balance,
        /// Block timestamp when the vault was created.
        pub created_at: u64,
    }

    /// Internal representation of per-netuid risk parameters used by the vault.
    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct VaultContractParams {
        /// Minimum collateralization ratio required for new borrows.
        pub collateral_ratio: Ratio,
        /// Collateralization ratio below which the vault is liquidatable.
        pub liquidation_ratio: Ratio,
        /// Fee charged on the collateral sold during liquidation.
        pub liquidation_fee: Ratio,
    }

    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    /// External per-netuid config uses basis points for ratio fields, where `100 = 1%`.
    pub struct VaultContractParamsConfig {
        /// Collateral ratio in basis points (100 = 1%).
        pub collateral_ratio: u32,
        /// Liquidation ratio in basis points (100 = 1%).
        pub liquidation_ratio: u32,
        /// Liquidation fee in basis points (100 = 1%).
        pub liquidation_fee: u32,
    }

    /// A queued parameter update awaiting timelock expiry before it can be executed.
    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PendingContractParamsUpdate {
        /// The scheduled per-netuid parameter config awaiting execution.
        pub params: VaultContractParamsConfig,
        /// Earliest block timestamp at which the update may be executed.
        pub execute_after: u64,
    }

    /// Internal representation of contract-wide (netuid-independent) parameters.
    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct VaultGlobalParams {
        /// Contract-wide transaction fee on liquidation collateral.
        pub transaction_fee: Ratio,
        /// Default liquidation auction duration in milliseconds.
        pub auction_duration_ms: u64,
        /// Maximum acceptable age of an oracle price, in milliseconds.
        pub max_oracle_age_ms: u64,
        /// Native TAO fee charged to open a vault (spam deterrent).
        pub vault_creation_fee: Balance,
    }

    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    /// External global config uses basis points for the fee field, where `100 = 1%`.
    pub struct VaultGlobalParamsConfig {
        /// Transaction fee in basis points (100 = 1%).
        pub transaction_fee: u32,
        /// Default liquidation auction duration in milliseconds.
        pub auction_duration_ms: u64,
        /// Maximum acceptable age of an oracle price, in milliseconds.
        pub max_oracle_age_ms: u64,
        /// Native TAO fee charged to open a vault (spam deterrent).
        pub vault_creation_fee: Balance,
    }

    /// A queued global-parameter update awaiting timelock expiry.
    #[derive(Debug, Copy, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PendingGlobalParamsUpdate {
        /// The scheduled global parameter config awaiting execution.
        pub params: VaultGlobalParamsConfig,
        /// Earliest block timestamp at which the update may be executed.
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

        /// Number of vaults currently in active liquidation auctions.
        /// Used as a fast guard — operations that conflict with liquidation
        /// (claim_excess_alpha, set_vault_hotkey, transfer_native_to_treasury)
        /// are blocked while this is non-zero.
        active_liquidation_count: u32,
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
    }

    /// Emitted when borrowed tUSDT is repaid.
    #[ink(event)]
    pub struct TokensRepaid {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        vault_id: u32,
        amount: Balance,
    }

    /// Emitted when per-netuid contract parameters are updated after timelock expiry.
    #[ink(event)]
    pub struct ContractParamsUpdated {
        params: VaultContractParamsConfig,
    }

    /// Emitted when a per-netuid parameter update is scheduled (timelock started).
    #[ink(event)]
    pub struct ContractParamsUpdateScheduled {
        params: VaultContractParamsConfig,
        execute_after: u64,
    }

    /// Emitted when a pending per-netuid parameter update is cancelled.
    #[ink(event)]
    pub struct ContractParamsUpdateCancelled {
        params: VaultContractParamsConfig,
    }

    /// Emitted when a global parameter update is scheduled (timelock started).
    #[ink(event)]
    pub struct GlobalParamsUpdateScheduled {
        params: VaultGlobalParamsConfig,
        execute_after: u64,
    }

    /// Emitted when global parameters are updated after timelock expiry.
    #[ink(event)]
    pub struct GlobalParamsUpdated {
        params: VaultGlobalParamsConfig,
    }

    /// Emitted when a pending global parameter update is cancelled.
    #[ink(event)]
    pub struct GlobalParamsUpdateCancelled {
        params: VaultGlobalParamsConfig,
    }

    /// Emitted when the governance address is changed.
    #[ink(event)]
    pub struct VaultGovernanceUpdated {
        #[ink(topic)]
        previous_governance: AccountId,
        #[ink(topic)]
        new_governance: AccountId,
    }

    /// Emitted when the treasury address is changed.
    #[ink(event)]
    pub struct VaultTreasuryUpdated {
        #[ink(topic)]
        previous_treasury: AccountId,
        #[ink(topic)]
        new_treasury: AccountId,
    }

    /// Emitted when the platform address is changed.
    #[ink(event)]
    pub struct VaultPlatformUpdated {
        #[ink(topic)]
        previous_platform: AccountId,
        #[ink(topic)]
        new_platform: AccountId,
    }

    /// Emitted when governance transfers the ERC20 token controller to a new account.
    #[ink(event)]
    pub struct TokenControllerUpdated {
        #[ink(topic)]
        new_controller: AccountId,
    }

    /// Emitted when the stored auction contract address is updated by governance.
    #[ink(event)]
    pub struct AuctionAddressUpdated {
        #[ink(topic)]
        old_auction: AccountId,
        #[ink(topic)]
        new_auction: AccountId,
    }

    /// Emitted when the stored oracle contract address is updated by governance.
    #[ink(event)]
    pub struct OracleAddressUpdated {
        #[ink(topic)]
        old_oracle: AccountId,
        #[ink(topic)]
        new_oracle: AccountId,
    }

    /// Emitted when the stored token contract address is updated by governance.
    #[ink(event)]
    pub struct TokenAddressUpdated {
        #[ink(topic)]
        old_token: AccountId,
        #[ink(topic)]
        new_token: AccountId,
    }

    /// Emitted when the contract is paused.
    #[ink(event)]
    pub struct Paused {}

    /// Emitted when the contract is unpaused.
    #[ink(event)]
    pub struct Unpaused {}

    /// Emitted when surplus TUSDT held by the vault is claimed to the treasury.
    #[ink(event)]
    pub struct SurplusTusdtClaimed {
        #[ink(topic)]
        recipient: AccountId,
        amount: Balance,
    }

    /// Emitted when excess alpha emissions are claimed and converted to native TAO.
    #[ink(event)]
    pub struct ExcessAlphaClaimed {
        #[ink(topic)]
        netuid: u16,
        excess_alpha: Balance,
        tao_received: Balance,
    }

    /// Emitted when the vault's staking hotkey is changed by governance.
    #[ink(event)]
    pub struct VaultHotkeyChanged {
        #[ink(topic)]
        old_hotkey: AccountId,
        #[ink(topic)]
        new_hotkey: AccountId,
    }

    /// Emitted when governance transfers native TAO from the vault to the treasury.
    #[ink(event)]
    pub struct NativeTransferredToTreasury {
        #[ink(topic)]
        amount: Balance,
    }

    /// Emitted when a netuid is added to or removed from the approved collateral set.
    #[ink(event)]
    pub struct NetuidApproved {
        #[ink(topic)]
        netuid: u16,
        approved: bool,
    }

    /// Emitted when a liquidation auction is created for an underwater vault.
    #[ink(event)]
    pub struct LiquidationAuctionCreated {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        vault_id: u32,
        #[ink(topic)]
        auction_id: u32,
    }

    /// Emitted when a liquidation auction is settled.
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
        /// No vault exists for the given (owner, vault_id) pair.
        VaultNotFound,
        /// Collateral amount is zero or insufficient for the operation.
        InsufficientCollateral,
        /// The caller does not own the specified vault.
        NotVaultOwner,
        /// A token or native TAO transfer failed.
        TransferFailed,
        /// The caller does not hold enough TUSDT balance.
        InsufficientTokenBalance,
        /// Transaction fee exceeds 100%.
        InvalidTransactionFee,
        /// Vault still has outstanding borrowed tokens that must be repaid first.
        TokenBorrowedNotZero,
        /// A ratio parameter failed validation bounds.
        InvalidRatio,
        /// Auction duration is outside the allowed [60 s, 7 d] range.
        InvalidAuctionDuration,
        /// The resulting debt would exceed the maximum allowed by the collateral ratio.
        CollateralRatioExceeded,
        /// The resulting debt would exceed the liquidation threshold.
        LiquidationRatioExceeded,
        /// Repayment amount exceeds the vault's current debt balance.
        RepayAmountTooHigh,
        /// The vault is currently in liquidation.
        VaultInLiquidation,
        /// The vault does not meet the liquidation criteria (debt not above threshold).
        NotLiquidatable,
        /// A liquidation auction already exists for this vault.
        LiquidationAuctionExists,
        /// Cross-contract call to the auction contract failed.
        AuctionContractCallFailed,
        /// No auction found for the given ID.
        AuctionNotFound,
        /// The auction has not yet been finalized.
        AuctionNotFinalized,
        /// Arithmetic overflow, underflow, or conversion failure.
        ArithmeticError,
        /// The caller is not the governance address.
        NotGovernance,
        /// The caller is neither the governance nor the platform address.
        NotGovernanceOrPlatform,
        /// The contract is paused.
        ContractPaused,
        /// Cross-contract call to the oracle failed.
        OracleCallFailed,
        /// No price data available from the oracle.
        OraclePriceUnavailable,
        /// The oracle price data has exceeded the maximum allowable age.
        OraclePriceStale,
        /// The max oracle age parameter is invalid (e.g., zero).
        InvalidOracleMaxAge,
        /// No pending parameter update for the given netuid.
        NoPendingContractParamsUpdate,
        /// No pending global-parameter update.
        NoPendingGlobalParamsUpdate,
        /// The timelock for the pending parameter update has not yet expired.
        ContractParamsUpdateTimelockActive,
        /// Chain extension call failed at the node level.
        ChainExtensionFailed,
        /// No alpha stake found for the vault's configured (hotkey, netuid).
        NoAlphaStakeFound,
        /// The caller-forwarded stake pull failed: the caller may lack sufficient stake
        /// under the vault's hotkey on that subnet, the amount may be below the chain's
        /// minimum, the subnet's TransferToggle may be off, or the runtime may not
        /// support the caller-forwarded chain-extension call.
        StakeTransferFailed,
        /// The specified netuid is not in the approved set.
        UnapprovedNetuid,
        /// Cross-contract call to the ERC20 token contract failed.
        TokenContractCallFailed,
        /// The caller did not transfer enough native TAO for the vault creation fee.
        VaultCreationFeeNotMet,
        /// Operation blocked because one or more vaults are in active liquidation auctions.
        ActiveLiquidationsExist,
        /// Too many netuids passed to `set_vault_hotkey` (max 32).
        TooManyNetuids,
        /// Cannot remove a netuid that still has active collateral positions.
        NetuidHasPositions,
    }

    /// Result type for alpha-vault operations, wrapping the contract-specific `Error` enum.
    pub type Result<T> = core::result::Result<T, Error>;

    /// Maximum number of netuids that can be passed to `set_vault_hotkey` in one call.
    const MAX_NETUIDS_PER_SET_HOTKEY: usize = 32;

    impl TusdtVaultAlpha {
        /// Initializes the alpha vault. `hotkey` is the single staking position for this
        /// vault instance. `oracle_netuid` is the subnet whose registered neurons may
        /// submit oracle prices. Governance must call `set_approved_netuid` to enable
        /// vault creation for specific subnets. The deployer becomes the initial governance.
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
                active_liquidation_count: 0,
            }
        }

        /// Initializes an alpha vault reusing an already-deployed TUSDT token contract.
        /// Unlike `new`, this does NOT spawn a new ERC20 — it takes `token_address`
        /// as an existing contract. The deployer becomes the initial governance.
        /// New auction and oracle child contracts are still spawned.
        ///
        /// Use this constructor when upgrading: the existing ERC20 (with balances,
        /// allowances, supply, and minter set) is preserved across the upgrade, and a
        /// new vault instance takes over control after governance transfers the token
        /// controller via `set_token_controller`.
        #[ink(constructor)]
        pub fn new_upgrade(
            treasury: AccountId,
            token_address: AccountId,
            auction_code_hash: Hash,
            oracle_code_hash: Hash,
            oracle_netuid: u16,
            hotkey: AccountId,
        ) -> Self {
            let governance = Self::env().caller();
            let contract_account = Self::env().account_id();

            let token = TusdtErc20Ref::from_account_id(token_address);
            let auction = TusdtAuctionRef::new(contract_account, governance, token_address)
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
                active_liquidation_count: 0,
            }
        }

        /// Initializes an alpha vault reusing already-deployed token, auction, and oracle
        /// contracts. Unlike `new`, this does NOT spawn any child contracts — it takes
        /// existing addresses for all three. The deployer becomes the initial governance.
        ///
        /// Use this constructor for a full state-preserving upgrade where the auction and
        /// oracle are retained alongside the token. Before calling
        /// `update_governance` on the new vault, governance MUST call
        /// `set_controller` on the existing auction and oracle contracts to transfer
        /// their controller role to this vault, otherwise `update_governance`'s
        /// `sync_child_governance` will fail (auction/oracle `update_governance` is
        /// controller-gated).
        #[ink(constructor)]
        pub fn new_upgrade_preserve_all(
            treasury: AccountId,
            token_address: AccountId,
            auction_address: AccountId,
            oracle_address: AccountId,
            hotkey: AccountId,
        ) -> Self {
            let governance = Self::env().caller();

            let token = TusdtErc20Ref::from_account_id(token_address);
            let auction = TusdtAuctionRef::from_account_id(auction_address);
            let oracle = TusdtOracleRef::from_account_id(oracle_address);

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
                active_liquidation_count: 0,
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
                .insert(netuid, &PendingContractParamsUpdate { params, execute_after });

            self.env().emit_event(ContractParamsUpdateScheduled { params, execute_after });

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

            self.env().emit_event(ContractParamsUpdated { params: pending.params });

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

            self.env().emit_event(ContractParamsUpdateCancelled { params: pending.params });

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
            self.pending_global_params_update =
                Some(PendingGlobalParamsUpdate { params: config, execute_after });

            self.env().emit_event(GlobalParamsUpdateScheduled { params: config, execute_after });

            Ok(())
        }

        /// Executes the currently scheduled global-parameter update once its timelock has elapsed.
        #[ink(message)]
        pub fn execute_global_params_update(&mut self) -> Result<()> {
            let pending =
                self.pending_global_params_update.ok_or(Error::NoPendingGlobalParamsUpdate)?;
            if self.env().block_timestamp() < pending.execute_after {
                return Err(Error::ContractParamsUpdateTimelockActive);
            }

            self.global_params = Self::global_params_from_config(pending.params)?;
            self.pending_global_params_update = None;

            self.env().emit_event(GlobalParamsUpdated { params: pending.params });

            Ok(())
        }

        /// Cancels the currently scheduled global-parameter update. Governance only.
        #[ink(message)]
        pub fn cancel_global_params_update(&mut self) -> Result<()> {
            self.ensure_governance()?;

            let pending = self
                .pending_global_params_update
                .take()
                .ok_or(Error::NoPendingGlobalParamsUpdate)?;

            self.env().emit_event(GlobalParamsUpdateCancelled { params: pending.params });

            Ok(())
        }

        /// Transfers the governance role to a new address and propagates the change
        /// to child auction and oracle contracts. Only governance may call.
        #[ink(message)]
        pub fn update_governance(&mut self, new_governance: AccountId) -> Result<()> {
            self.ensure_governance()?;

            self.sync_child_governance(new_governance)?;

            let previous_governance = self.governance;
            self.governance = new_governance;

            self.env().emit_event(VaultGovernanceUpdated { previous_governance, new_governance });

            Ok(())
        }

        /// Updates the treasury address. Only governance may call.
        #[ink(message)]
        pub fn update_treasury(&mut self, new_treasury: AccountId) -> Result<()> {
            self.ensure_governance()?;

            let previous_treasury = self.treasury;
            self.treasury = new_treasury;

            self.env().emit_event(VaultTreasuryUpdated { previous_treasury, new_treasury });

            Ok(())
        }

        /// Updates the platform address. Only governance may call.
        #[ink(message)]
        pub fn update_platform(&mut self, new_platform: AccountId) -> Result<()> {
            self.ensure_governance()?;

            let previous_platform = self.platform;
            self.platform = new_platform;

            self.env().emit_event(VaultPlatformUpdated { previous_platform, new_platform });

            Ok(())
        }

        /// Transfers the ERC20 token controller role to a new account. Governance only.
        /// Needed during upgrades to hand token control from the old vault to a newly
        /// deployed vault. Calls through to the ERC20 contract's `set_controller`,
        /// which also handles minter-set migration.
        #[ink(message)]
        pub fn set_token_controller(&mut self, new_controller: AccountId) -> Result<()> {
            self.ensure_governance()?;
            self.token
                .set_controller(new_controller)
                .map_err(|_| Error::TokenContractCallFailed)?;
            self.env().emit_event(TokenControllerUpdated { new_controller });
            Ok(())
        }

        /// Updates the stored auction contract address. Governance only.
        /// Used when the auction is redeployed or replaced during an upgrade.
        #[ink(message)]
        pub fn update_auction_address(&mut self, new_auction: AccountId) -> Result<()> {
            self.ensure_governance()?;
            let old_auction = self.auction.to_account_id();
            self.auction = TusdtAuctionRef::from_account_id(new_auction);
            self.env().emit_event(AuctionAddressUpdated { old_auction, new_auction });
            Ok(())
        }

        /// Updates the stored oracle contract address. Governance only.
        /// Used when the oracle is redeployed or replaced during an upgrade.
        #[ink(message)]
        pub fn update_oracle_address(&mut self, new_oracle: AccountId) -> Result<()> {
            self.ensure_governance()?;
            let old_oracle = self.oracle.to_account_id();
            self.oracle = TusdtOracleRef::from_account_id(new_oracle);
            self.env().emit_event(OracleAddressUpdated { old_oracle, new_oracle });
            Ok(())
        }

        /// Updates the stored token (ERC20) contract address. Governance only.
        /// Used when the token contract is redeployed and the vault needs to
        /// reference the new instance.
        #[ink(message)]
        pub fn update_token_address(&mut self, new_token: AccountId) -> Result<()> {
            self.ensure_governance()?;
            let old_token = self.token.to_account_id();
            self.token = TusdtErc20Ref::from_account_id(new_token);
            self.env().emit_event(TokenAddressUpdated { old_token, new_token });
            Ok(())
        }

        /// Pauses the contract, blocking deposits, borrowing, repayments, collateral
        /// operations, liquidation triggers, and interest accrual.
        /// Callable by governance or platform (emergency halt).
        #[ink(message)]
        pub fn pause(&mut self) -> Result<()> {
            self.ensure_governance_or_platform()?;

            self.paused = true;
            self.env().emit_event(Paused {});

            Ok(())
        }

        /// Unpauses the contract, restoring normal operations. Only governance may call.
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
                // Refuse removal if any vaults still hold collateral on this netuid.
                let total = self.netuid_total_collateral.get(netuid).unwrap_or_default();
                if total > 0 {
                    return Err(Error::NetuidHasPositions);
                }
                self.approved_netuids.remove(netuid);
            }
            self.env().emit_event(NetuidApproved { netuid, approved });
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
            self.token.transfer(self.treasury, amount).map_err(|_| Error::TransferFailed)?;

            self.env().emit_event(SurplusTusdtClaimed { recipient: self.treasury, amount });

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

            // Block claiming excess while any liquidation auction is in progress —
            // the unstaked alpha from the liquidated vault has not yet been settled.
            if self.active_liquidation_count > 0 {
                return Err(Error::ActiveLiquidationsExist);
            }

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

            let excess = current_stake.checked_sub(netuid_total).ok_or(Error::ArithmeticError)?;

            // Snapshot native TAO balance before unstaking.
            let balance_before = self.env().balance();

            // Unstake excess alpha to native TAO via chain extension.
            self.env()
                .extension()
                .remove_stake(self.vault_hotkey, netuid, excess)
                .map_err(|_| Error::ChainExtensionFailed)?;

            // Read the actual TAO received from the balance change.
            let balance_after = self.env().balance();
            let tao_received =
                balance_after.checked_sub(balance_before).ok_or(Error::ArithmeticError)?;

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

        /// Moves all alpha stake from the current vault hotkey to `new_hotkey` across
        /// the specified netuids, then updates the stored hotkey. Governance only.
        /// Reverts if any liquidation auction is active.
        ///
        /// Uses chain extension `move_stake` (func 5) which transfers the staking
        /// position from one hotkey to another on the same netuid. The contract
        /// remains the coldkey throughout — only the hotkey identity changes.
        ///
        /// Netuids with no stake are silently skipped. The hotkey is only updated
        /// after all moves succeed (the extrinsic is atomic — partial success rolls back).
        #[ink(message)]
        pub fn set_vault_hotkey(&mut self, new_hotkey: AccountId, netuids: Vec<u16>) -> Result<()> {
            self.ensure_governance()?;
            self.ensure_no_active_liquidations()?;

            if netuids.len() > MAX_NETUIDS_PER_SET_HOTKEY {
                return Err(Error::TooManyNetuids);
            }

            let old_hotkey = self.vault_hotkey;

            for netuid in netuids {
                let stake = match self.get_contract_stake(netuid) {
                    Ok(s) => s,
                    Err(Error::NoAlphaStakeFound) => continue,
                    Err(e) => return Err(e),
                };
                if stake > 0 {
                    self.env()
                        .extension()
                        .move_stake(old_hotkey, new_hotkey, netuid, netuid, stake)
                        .map_err(|_| Error::ChainExtensionFailed)?;
                }
            }

            self.vault_hotkey = new_hotkey;

            self.env().emit_event(VaultHotkeyChanged { old_hotkey, new_hotkey });

            Ok(())
        }

        /// Transfers the vault contract's entire native TAO balance to the treasury.
        /// Governance only. Reverts if any liquidation auction is active (native TAO
        /// from liquidation must remain in the contract until settlement).
        ///
        /// This is a general-purpose sweep for TAO that accumulates in the contract
        /// outside the normal liquidation/claim-excess flows (e.g., dust, overpayments).
        #[ink(message)]
        pub fn transfer_native_to_treasury(&mut self) -> Result<()> {
            self.ensure_governance()?;
            self.ensure_no_active_liquidations()?;

            let balance = self.env().balance();
            // Reserve the chain's existential deposit to prevent the contract
            // account from being reaped.
            let ed = self.env().minimum_balance();
            let transfer_amount = balance.saturating_sub(ed);
            if transfer_amount > 0 {
                self.env()
                    .transfer(self.treasury, transfer_amount)
                    .map_err(|_| Error::TransferFailed)?;
            }

            self.env().emit_event(NativeTransferredToTreasury { amount: transfer_amount });

            Ok(())
        }

        // ── Alpha vault lifecycle ───────────────────────────────────────

        /// Creates a new alpha-backed vault for the caller on the specified subnet by
        /// atomically pulling `amount` of the CALLER's alpha stake (held under this
        /// vault's hotkey) into the contract's custody via the caller-forwarded
        /// `caller_transfer_stake` chain extension. No prior deposit intent or external
        /// `transfer_stake` extrinsic is needed.
        ///
        /// A vault creation fee (in native TAO) is charged to deter spam. The fee amount
        /// is governed by `VaultGlobalParams::vault_creation_fee`. Any excess transferred
        /// value is refunded to the caller.
        #[ink(message, payable)]
        pub fn create_alpha_vault(&mut self, amount: Balance, netuid: u16) -> Result<u32> {
            self.ensure_not_paused()?;
            self.ensure_approved_netuid(netuid)?;
            if amount == 0 {
                return Err(Error::InsufficientCollateral);
            }

            let caller = self.env().caller();
            let transferred = self.env().transferred_value();
            let vault_creation_fee = self.global_params.vault_creation_fee;

            if transferred < vault_creation_fee {
                return Err(Error::VaultCreationFeeNotMet);
            }

            // Pull the caller's stake into the contract's coldkey.
            self.env()
                .extension()
                .caller_transfer_stake(
                    self.env().account_id(),
                    self.vault_hotkey,
                    netuid,
                    netuid,
                    amount,
                )
                .map_err(|_| Error::StakeTransferFailed)?;

            let (_, _) = self.ensure_collateral_bounds(netuid, 0, amount)?;
            let netuid_total = self.netuid_total_collateral.get(netuid).unwrap_or_default();
            let projected_netuid =
                netuid_total.checked_add(amount).ok_or(Error::ArithmeticError)?;

            let timestamp = self.env().block_timestamp();

            let vault_id = self.vault_count.get(caller).unwrap_or(0);
            let vault = Vault {
                id: vault_id,
                owner: caller,
                netuid,
                collateral_balance: amount,
                borrowed_token_balance: 0,
                created_at: timestamp,
            };

            self.save_vault(caller, vault_id, &vault)?;
            self.vault_keys.push(&(caller, vault_id));
            self.total_collateral_balance =
                self.total_collateral_balance.checked_add(amount).ok_or(Error::ArithmeticError)?;
            self.netuid_total_collateral.insert(netuid, &projected_netuid);

            let next_id = vault_id.checked_add(1).ok_or(Error::ArithmeticError)?;
            self.vault_count.insert(caller, &next_id);

            // Transfer vault creation fee to treasury.
            if vault_creation_fee > 0 {
                self.env()
                    .transfer(self.treasury, vault_creation_fee)
                    .map_err(|_| Error::TransferFailed)?;
            }
            // Refund excess if caller sent more than the fee.
            let excess =
                transferred.checked_sub(vault_creation_fee).ok_or(Error::ArithmeticError)?;
            if excess > 0 {
                self.env().transfer(caller, excess).map_err(|_| Error::TransferFailed)?;
            }

            self.env().emit_event(VaultCreated { owner: caller, vault_id, amount });

            Ok(vault_id)
        }

        /// Tops up an alpha vault's collateral by atomically pulling `amount` of the
        /// CALLER's alpha stake (held under this vault's hotkey) into the contract's
        /// custody. Exactly `amount` is credited to the caller's vault — unattributed
        /// stake growth (emissions) remains claimable only by governance via
        /// `claim_excess_alpha`.
        #[ink(message)]
        pub fn add_alpha_collateral(&mut self, vault_id: u32, amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;

            let (caller, mut vault) = self.load_caller_vault(vault_id)?;

            // Pull the caller's stake into the contract's coldkey (destination is
            // always this contract — never user-supplied).
            self.env()
                .extension()
                .caller_transfer_stake(
                    self.env().account_id(),
                    self.vault_hotkey,
                    vault.netuid,
                    vault.netuid,
                    amount,
                )
                .map_err(|_| Error::StakeTransferFailed)?;

            let (projected_vault, _) =
                self.ensure_collateral_bounds(vault.netuid, vault.collateral_balance, amount)?;

            let netuid_total = self.netuid_total_collateral.get(vault.netuid).unwrap_or_default();
            vault.collateral_balance = projected_vault;
            self.total_collateral_balance =
                self.total_collateral_balance.checked_add(amount).ok_or(Error::ArithmeticError)?;
            let projected_netuid =
                netuid_total.checked_add(amount).ok_or(Error::ArithmeticError)?;
            self.netuid_total_collateral.insert(vault.netuid, &projected_netuid);
            self.save_vault(caller, vault_id, &vault)?;

            self.env().emit_event(CollateralAdded { owner: caller, vault_id, amount });

            Ok(())
        }

        /// Borrows TUSDT against the vault's alpha collateral. Mints the full
        /// amount to the caller. Validates that the resulting borrowed balance
        /// does not exceed the maximum allowed by the collateral ratio.
        #[ink(message)]
        pub fn borrow_token(&mut self, vault_id: u32, amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;

            let (caller, mut vault) = self.load_caller_vault(vault_id)?;

            if amount.eq(&0) {
                self.save_vault(caller, vault_id, &vault)?;
                return Ok(());
            }

            let price = self.current_collateral_price(vault.netuid)?;

            let max_borrow =
                self.max_borrow_allowed(vault.netuid, price, vault.collateral_balance)?;
            let projected_borrowed =
                vault.borrowed_token_balance.checked_add(amount).ok_or(Error::ArithmeticError)?;
            if projected_borrowed > max_borrow {
                return Err(Error::CollateralRatioExceeded);
            }

            vault.borrowed_token_balance = projected_borrowed;
            self.save_vault(caller, vault_id, &vault)?;

            if amount > 0 {
                self.token.mint(caller, amount).map_err(|_| Error::TransferFailed)?;
            }

            self.env().emit_event(TokensBorrowed { owner: caller, vault_id, amount });

            Ok(())
        }

        /// Repays borrowed TUSDT on an alpha vault. Burns the caller's tokens
        /// and decrements the borrowed balance. No transaction fee is charged
        /// on repayments.
        #[ink(message)]
        pub fn repay_token(&mut self, vault_id: u32, amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;

            let (caller, mut vault) = self.load_caller_vault(vault_id)?;

            if amount.eq(&0) {
                self.save_vault(caller, vault_id, &vault)?;
                return Ok(());
            }
            if amount > vault.borrowed_token_balance {
                return Err(Error::RepayAmountTooHigh);
            }
            self.ensure_token_balance_at_least(caller, amount)?;

            self.token.burn(caller, amount).map_err(|_| Error::TransferFailed)?;

            vault.borrowed_token_balance =
                vault.borrowed_token_balance.checked_sub(amount).ok_or(Error::ArithmeticError)?;
            self.save_vault(caller, vault_id, &vault)?;

            self.env().emit_event(TokensRepaid { owner: caller, vault_id, amount });

            Ok(())
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

            let projected_collateral =
                vault.collateral_balance.checked_sub(amount).ok_or(Error::ArithmeticError)?;
            if vault.borrowed_token_balance > 0 {
                let price = self.current_collateral_price(vault.netuid)?;
                let max_borrow_after_release =
                    self.max_borrow_allowed(vault.netuid, price, projected_collateral)?;
                if vault.borrowed_token_balance > max_borrow_after_release {
                    return Err(Error::CollateralRatioExceeded);
                }
            }

            // Transfer stake back to the user via chain extension.
            self.env()
                .extension()
                .transfer_stake(dest_coldkey, self.vault_hotkey, vault.netuid, vault.netuid, amount)
                .map_err(|_| Error::ChainExtensionFailed)?;

            self.total_collateral_balance =
                self.total_collateral_balance.checked_sub(amount).ok_or(Error::ArithmeticError)?;
            let netuid_total = self.netuid_total_collateral.get(vault.netuid).unwrap_or_default();
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
        /// standard ascending-bid auction with the recovered TAO.
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
            let price = self.current_collateral_price(vault.netuid)?;

            if !self.is_liquidatable(price, &vault)? {
                return Err(Error::NotLiquidatable);
            }

            // Unstake all alpha collateral to recover TAO into the contract's balance.
            // Read actual balance change to avoid drift from the formula-based estimate.
            let collateral_amount = vault.collateral_balance;
            let balance_before = self.env().balance();
            self.env()
                .extension()
                .remove_stake(self.vault_hotkey, vault.netuid, collateral_amount)
                .map_err(|_| Error::TransferFailed)?;
            let balance_after = self.env().balance();
            let tao_received =
                balance_after.checked_sub(balance_before).ok_or(Error::ArithmeticError)?;
            if tao_received == 0 {
                return Err(Error::TransferFailed);
            }

            // Zero out the vault's collateral immediately — the alpha has been
            // unstaked and the TAO now sits in the contract's balance.
            vault.collateral_balance = 0;
            self.total_collateral_balance = self
                .total_collateral_balance
                .checked_sub(collateral_amount)
                .ok_or(Error::ArithmeticError)?;
            let netuid_total = self.netuid_total_collateral.get(vault.netuid).unwrap_or_default();
            self.netuid_total_collateral.insert(
                vault.netuid,
                &netuid_total.checked_sub(collateral_amount).ok_or(Error::ArithmeticError)?,
            );
            self.save_vault(owner, vault_id, &vault)?;

            // Now the TAO is in the contract's balance. Create a standard auction
            // with the actual TAO amount received.
            let min_bid = self.liquidation_min_bid(vault.netuid, vault.borrowed_token_balance)?;
            let auction_id = self
                .auction
                .create_auction(
                    owner,
                    vault_id,
                    tao_received,
                    vault.borrowed_token_balance,
                    min_bid,
                    price,
                    Some(self.global_params.auction_duration_ms),
                )
                .map_err(|_| Error::AuctionContractCallFailed)?;

            self.liquidation_auctions.insert((owner, vault_id), &auction_id);

            // Increment the active liquidation counter.
            self.active_liquidation_count =
                self.active_liquidation_count.checked_add(1).ok_or(Error::ArithmeticError)?;

            self.env().emit_event(LiquidationAuctionCreated { owner, vault_id, auction_id });

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
            let auction_id =
                self.liquidation_auctions.get((owner, vault_id)).ok_or(Error::AuctionNotFound)?;

            let auction = self.auction.get_auction(auction_id).ok_or(Error::AuctionNotFound)?;

            if !auction.is_finalized || auction.highest_bidder.is_none() {
                return Err(Error::AuctionNotFinalized);
            }

            let winner = auction.highest_bidder.ok_or(Error::AuctionNotFinalized)?;
            let winning_bid = auction.highest_bid;

            let mut vault = self.load_vault(owner, vault_id)?;
            let collateral_sold = auction.collateral_balance;
            let transaction_fee = self.calculate_transaction_fee(collateral_sold)?;
            let debt_cleared = auction.debt_balance;

            // ── Effects first (CEI): collateral was already zeroed in
            //     trigger_liquidation_auction. Only vault debt state and the
            //     liquidation marker need cleanup here.
            vault.borrowed_token_balance = vault
                .borrowed_token_balance
                .checked_sub(debt_cleared)
                .ok_or(Error::ArithmeticError)?;
            self.save_vault(owner, vault_id, &vault)?;
            self.liquidation_auctions.remove((owner, vault_id));

            // ── Interactions: external calls and transfers.
            self.auction
                .transfer_winning_bid(auction_id, self.env().account_id())
                .map_err(|_| Error::AuctionContractCallFailed)?;

            let winner_collateral =
                collateral_sold.checked_sub(transaction_fee).ok_or(Error::ArithmeticError)?;

            if transaction_fee > 0 {
                self.transfer_transaction_fee_to_treasury(transaction_fee)?;
            }
            if winner_collateral > 0 && self.env().transfer(winner, winner_collateral).is_err() {
                return Err(Error::TransferFailed);
            }
            // Burn the full winning bid (all principal — no interest split).
            self.token
                .burn(self.env().account_id(), debt_cleared)
                .map_err(|_| Error::TransferFailed)?;

            // ── Effects: decrement counter only after all external calls succeed.
            //     This ensures the liquidation gate is not released prematurely
            //     if any interaction fails.
            self.active_liquidation_count =
                self.active_liquidation_count.checked_sub(1).ok_or(Error::ArithmeticError)?;

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

        /// Returns the vault record for the given owner and vault ID, or `None`.
        #[ink(message)]
        pub fn get_vault(&self, owner: AccountId, vault_id: u32) -> Option<Vault> {
            self.vaults.get((owner, vault_id))
        }

        /// Returns the TUSDT token contract address.
        #[ink(message)]
        pub fn get_token_address(&self) -> AccountId {
            self.token.to_account_id()
        }

        /// Returns the auction contract address.
        #[ink(message)]
        pub fn get_auction_address(&self) -> AccountId {
            self.auction.to_account_id()
        }

        /// Returns the oracle contract address.
        #[ink(message)]
        pub fn get_oracle_address(&self) -> AccountId {
            self.oracle.to_account_id()
        }

        /// Returns the current governance address.
        #[ink(message)]
        pub fn governance(&self) -> AccountId {
            self.governance
        }

        /// Returns the vault's staking hotkey — the single hotkey under which all
        /// subnet alpha collateral is staked for this vault instance.
        #[ink(message)]
        pub fn get_vault_hotkey(&self) -> AccountId {
            self.vault_hotkey
        }

        /// Returns the number of vaults currently in active liquidation auctions.
        #[ink(message)]
        pub fn get_active_liquidation_count(&self) -> u32 {
            self.active_liquidation_count
        }

        /// Returns the current treasury address.
        #[ink(message)]
        pub fn treasury(&self) -> AccountId {
            self.treasury
        }

        /// Returns the current platform address.
        #[ink(message)]
        pub fn platform(&self) -> AccountId {
            self.platform
        }

        /// Returns `true` if the contract is currently paused.
        #[ink(message)]
        pub fn paused(&self) -> bool {
            self.paused
        }

        /// Returns the per-netuid contract parameters as an external bps-based config.
        #[ink(message)]
        pub fn get_contract_params(&self, netuid: u16) -> VaultContractParamsConfig {
            Self::contract_params_to_config(self.get_params(netuid))
        }

        /// Returns the global (contract-wide) parameters as an external bps-based config.
        #[ink(message)]
        pub fn get_global_params(&self) -> VaultGlobalParamsConfig {
            Self::global_params_to_config(self.global_params)
        }

        /// Returns the currently scheduled parameter update for a netuid, if any.
        #[ink(message)]
        pub fn get_pending_contract_params_update(
            &self,
            netuid: u16,
        ) -> Option<PendingContractParamsUpdate> {
            self.pending_contract_params_updates.get(netuid)
        }

        /// Returns the alpha collateral balance of a vault.
        #[ink(message)]
        pub fn get_vault_collateral_balance(
            &self,
            owner: AccountId,
            vault_id: u32,
        ) -> Option<Balance> {
            self.vaults.get((owner, vault_id)).map(|v| v.collateral_balance)
        }

        /// Returns the total alpha collateral across all vaults (all netuids).
        #[ink(message)]
        pub fn get_total_collateral_balance(&self) -> Balance {
            self.total_collateral_balance
        }

        /// Returns the total debt accrued by an owner across all their vaults.
        #[ink(message)]
        pub fn get_total_debt(&self, owner: AccountId) -> Balance {
            self.owner_total_debt.get(owner).unwrap_or_default()
        }

        /// Returns the TUSDT value of a vault's alpha collateral at current oracle prices.
        #[ink(message)]
        pub fn get_vault_collateral_value(
            &self,
            owner: AccountId,
            vault_id: u32,
        ) -> Result<Balance> {
            let vault = self.vaults.get((owner, vault_id)).ok_or(Error::VaultNotFound)?;
            let price = self.current_collateral_price(vault.netuid)?;
            Self::collateral_value(price, vault.collateral_balance)
        }

        /// Returns the maximum borrow amount for a vault given its current collateral
        /// and the configured collateral ratio for its netuid.
        #[ink(message)]
        pub fn get_max_borrow(&self, owner: AccountId, vault_id: u32) -> Result<Balance> {
            let vault = self.vaults.get((owner, vault_id)).ok_or(Error::VaultNotFound)?;
            let price = self.current_collateral_price(vault.netuid)?;
            let max = self.max_borrow_allowed(vault.netuid, price, vault.collateral_balance)?;

            Ok(max)
        }

        /// Returns the liquidation auction ID for a vault, if one exists.
        #[ink(message)]
        pub fn get_liquidation_auction_id(&self, owner: AccountId, vault_id: u32) -> Option<u32> {
            self.liquidation_auctions.get((owner, vault_id))
        }

        /// Returns the total number of vaults across all owners.
        #[ink(message)]
        pub fn get_total_vaults_count(&self) -> u32 {
            self.vault_keys.len()
        }

        /// Returns the number of vaults for a given owner.
        #[ink(message)]
        pub fn get_vaults_count(&self, owner: AccountId) -> u32 {
            self.vault_count.get(owner).unwrap_or_default()
        }

        /// Returns a paginated list of vaults for a given owner. Each page has up to 10 entries.
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
                let vault = self.vaults.get((owner, index)).ok_or(Error::VaultNotFound)?;
                vaults.push(vault);
            }

            Ok(vaults)
        }

        /// Returns a paginated list of all vaults across all owners. Each page has up to 10 entries.
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
                let key = self.vault_keys.get(index).ok_or(Error::VaultNotFound)?;
                let vault = self.vaults.get(key).ok_or(Error::VaultNotFound)?;
                vaults.push(vault);
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
                .get_stake_info_for_hotkey_coldkey_netuid(self.vault_hotkey, contract_addr, netuid)
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
            self.netuid_params.get(netuid).unwrap_or_else(Self::default_contract_params)
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
            tusdt_per_tao.checked_mul(alpha_to_tao).ok_or(Error::ArithmeticError)
        }

        /// Validates oracle price data: ensures a price exists, is non-stale (within
        /// `max_oracle_age_ms`), and is non-zero.
        pub(crate) fn validate_price_data(
            price_data: Option<PriceData>,
            now: u64,
            max_oracle_age_ms: u64,
        ) -> Result<PriceData> {
            let price_data = price_data.ok_or(Error::OraclePriceUnavailable)?;
            if price_data.committed_at > now {
                return Err(Error::OraclePriceUnavailable);
            }
            let age = now.checked_sub(price_data.committed_at).ok_or(Error::ArithmeticError)?;
            if age > max_oracle_age_ms {
                return Err(Error::OraclePriceStale);
            }
            if price_data.price.is_zero() {
                return Err(Error::OraclePriceUnavailable);
            }
            Ok(price_data)
        }

        /// Syncs an owner's aggregate debt tracker when a vault's debt changes.
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

        /// Applies a repayment amount to a vault's debt, splitting it into principal
        /// Calculates the transaction fee on a given amount using the global
        /// `transaction_fee` ratio (Balance = u64, fee in rao units).
        /// Only used during auction settlement.
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

        /// Ensures the given account holds at least `required_balance` TUSDT.
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

        /// Transfers native TAO transaction fee to the treasury.
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

        /// Ensures the caller is the governance address.
        #[inline]
        fn ensure_governance(&self) -> Result<()> {
            if self.env().caller() != self.governance {
                return Err(Error::NotGovernance);
            }
            Ok(())
        }

        /// Ensures the caller is the governance or platform address.
        #[inline]
        fn ensure_governance_or_platform(&self) -> Result<()> {
            let caller = self.env().caller();
            if caller != self.governance && caller != self.platform {
                return Err(Error::NotGovernanceOrPlatform);
            }
            Ok(())
        }

        /// Ensures the contract is not paused.
        #[inline]
        fn ensure_not_paused(&self) -> Result<()> {
            if self.paused {
                return Err(Error::ContractPaused);
            }
            Ok(())
        }

        /// Ensures the given netuid is in the approved set.
        #[inline]
        fn ensure_approved_netuid(&self, netuid: u16) -> Result<()> {
            if self.approved_netuids.get(netuid).is_none() {
                return Err(Error::UnapprovedNetuid);
            }
            Ok(())
        }

        /// Ensures no liquidation auctions are currently active.
        /// Used by governance operations (`claim_excess_alpha`, `set_vault_hotkey`,
        /// `transfer_native_to_treasury`) that must not run concurrently with liquidations.
        fn ensure_no_active_liquidations(&self) -> Result<()> {
            if self.active_liquidation_count > 0 {
                return Err(Error::ActiveLiquidationsExist);
            }
            Ok(())
        }

        /// Propagates governance change to child auction and oracle contracts.
        /// No-op in test builds where child contracts are not real instances.
        #[cfg(not(test))]
        fn sync_child_governance(&mut self, new_governance: AccountId) -> Result<()> {
            self.auction
                .update_governance(new_governance)
                .map_err(|_| Error::AuctionContractCallFailed)?;
            self.oracle.update_governance(new_governance).map_err(|_| Error::OracleCallFailed)?;
            Ok(())
        }

        #[cfg(test)]
        fn sync_child_governance(&mut self, _new_governance: AccountId) -> Result<()> {
            Ok(())
        }
    }

    #[cfg(test)]
    impl TusdtVaultAlpha {
        /// Test-only constructor: builds a vault with placeholder child-contract
        /// addresses so unit tests can run without deployed token/auction/oracle
        /// instances. The governance account also becomes maintainer and platform.
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
                active_liquidation_count: 0,
            }
        }

        /// Test-only: directly record the liquidation-auction mapping for
        /// (owner, vault_id) without going through the auction lifecycle.
        pub(crate) fn set_liquidation_auction_for_test(
            &mut self,
            owner: AccountId,
            vault_id: u32,
            auction_id: u32,
        ) {
            self.liquidation_auctions.insert((owner, vault_id), &auction_id);
        }

        /// Set the active liquidation counter directly (bypasses the normal
        /// trigger/settle flow) for testing liquidation-gated operations.
        pub(crate) fn set_active_liquidation_count_for_test(&mut self, count: u32) {
            self.active_liquidation_count = count;
        }
    }
}

#[cfg(test)]
mod tests;
