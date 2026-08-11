#![cfg_attr(not(feature = "std"), no_std, no_main)]
#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::enum_variant_names)]

pub use self::lending_pool::{
    AlphaMarketParams, AlphaMarketParamsConfig, InterestRateParams, InterestRateParamsConfig,
    PoolGlobalParams, PoolGlobalParamsConfig, TusdtLendingPool, TusdtLendingPoolRef,
};

#[ink::contract(env = tusdt_env::CustomEnvironment)]
mod lending_pool {
    use core::cmp::min;
    use ink::env::call::FromAccountId;
    use ink::prelude::vec::Vec;
    use ink::storage::{Mapping, StorageVec};
    use ink::ToAccountId;

    use tusdt_erc20::TusdtErc20Ref;
    use tusdt_oracle::TusdtOracleRef;
    use tusdt_primitives::Ratio;

    const PAGE_SIZE: u32 = 10;
    pub(crate) const PARAMS_TIMELOCK_MS: u64 = 24 * 60 * 60 * 1_000;
    #[allow(dead_code)]
    pub(crate) const MIN_STAKE: Balance = 100_000;

    /// Which asset a user is interacting with.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    #[allow(dead_code)]
    pub enum Asset {
        TAO = 0,
        TUSDT = 1,
    }

    /// Per-market runtime accrual state. Markets 0 (TAO) and 1 (TUSDT) are supply+borrow
    /// markets with interest accrual. Markets 2+ are alpha collateral-only markets (one per
    /// approved subnet); their MarketState exists but accrual is a no-op.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct MarketState {
        /// Total underlying supplied (principal deposits only — not including accrued interest).
        pub total_supplied: Balance,
        /// Total outstanding debt in underlying units (includes accrued interest).
        pub total_debt: Balance,
        /// Accumulator for per-user scaled debt: user_debt = scaled_debt × borrow_index.
        /// Starts at 1.0 (1e18); grows at the borrow rate.
        pub borrow_index: Ratio,
        /// lToken exchange rate: underlying = ltoken_balance × exchange_rate.
        /// Starts at 1.0; grows at the supply rate.
        pub exchange_rate: Ratio,
        /// Protocol share of accrued interest, in underlying units.
        pub reserve_accrued: Balance,
        /// Timestamp (ms) of the last interest accrual.
        pub last_update: u64,
    }

    impl MarketState {
        fn new(now: u64) -> Self {
            Self {
                total_supplied: 0,
                total_debt: 0,
                borrow_index: Ratio::one(),
                exchange_rate: Ratio::one(),
                reserve_accrued: 0,
                last_update: now,
            }
        }
    }

    /// Per-(market, user) position.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct Position {
        /// lToken receipt balance held by the user (supply markets only).
        pub ltoken_balance: Balance,
        /// Scaled debt: actual_debt = scaled_debt × market.borrow_index.
        pub scaled_debt: Balance,
        /// Alpha principal units deposited (alpha markets only).
        /// Effective collateral = alpha_principal × netuid_yield_index.
        pub alpha_principal: Balance,
    }

    /// Per-market interest-rate parameters (internal, Ratio-based).
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct InterestRateParams {
        pub base_rate: Ratio,
        pub slope1: Ratio,
        pub slope2: Ratio,
        pub optimal_utilization: Ratio,
        pub reserve_factor: Ratio,
    }

    /// External config for interest-rate params (basis points).
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct InterestRateParamsConfig {
        pub base_rate: u32,
        pub slope1: u32,
        pub slope2: u32,
        pub optimal_utilization: u32,
        pub reserve_factor: u32,
    }

    /// Per-netuid alpha market parameters (internal, Ratio-based).
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct AlphaMarketParams {
        pub collateral_factor: Ratio,
        pub liquidation_threshold: Ratio,
        pub liquidation_bonus: Ratio,
        pub supply_cap: Balance,
    }

    /// External config for alpha market params (basis points).
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct AlphaMarketParamsConfig {
        pub collateral_factor: u32,
        pub liquidation_threshold: u32,
        pub liquidation_bonus: u32,
        pub supply_cap: Balance,
    }

    /// Global pool parameters (internal, Ratio-based).
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PoolGlobalParams {
        pub max_oracle_age_ms: u64,
        pub close_factor: Ratio,
        pub performance_fee: Ratio,
        pub supply_cap_tao: Balance,
        pub supply_cap_tusdt: Balance,
        pub borrow_cap_tao: Balance,
        pub borrow_cap_tusdt: Balance,
    }

    /// External config for global params.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PoolGlobalParamsConfig {
        pub max_oracle_age_ms: u64,
        pub close_factor: u32,
        pub performance_fee: u32,
        pub supply_cap_tao: Balance,
        pub supply_cap_tusdt: Balance,
        pub borrow_cap_tao: Balance,
        pub borrow_cap_tusdt: Balance,
    }

    /// Queued interest-rate parameter update awaiting timelock expiry.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PendingInterestParamsUpdate {
        pub params: InterestRateParamsConfig,
        pub execute_after: u64,
    }

    /// Queued alpha-market parameter update awaiting timelock expiry.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PendingAlphaParamsUpdate {
        pub params: AlphaMarketParamsConfig,
        pub execute_after: u64,
    }

    /// Queued global parameter update awaiting timelock expiry.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct PendingGlobalParamsUpdate {
        pub params: PoolGlobalParamsConfig,
        pub execute_after: u64,
    }

    // ── Config conversion helpers ──

    impl InterestRateParams {
        pub fn to_config(&self) -> InterestRateParamsConfig {
            InterestRateParamsConfig {
                base_rate: self.base_rate.to_basis_points().unwrap_or(0),
                slope1: self.slope1.to_basis_points().unwrap_or(0),
                slope2: self.slope2.to_basis_points().unwrap_or(0),
                optimal_utilization: self.optimal_utilization.to_basis_points().unwrap_or(0),
                reserve_factor: self.reserve_factor.to_basis_points().unwrap_or(0),
            }
        }
    }

    impl AlphaMarketParams {
        pub fn to_config(&self) -> AlphaMarketParamsConfig {
            AlphaMarketParamsConfig {
                collateral_factor: self.collateral_factor.to_basis_points().unwrap_or(0),
                liquidation_threshold: self.liquidation_threshold.to_basis_points().unwrap_or(0),
                liquidation_bonus: self.liquidation_bonus.to_basis_points().unwrap_or(0),
                supply_cap: self.supply_cap,
            }
        }
    }

    impl PoolGlobalParams {
        pub fn to_config(&self) -> PoolGlobalParamsConfig {
            PoolGlobalParamsConfig {
                max_oracle_age_ms: self.max_oracle_age_ms,
                close_factor: self.close_factor.to_basis_points().unwrap_or(0),
                performance_fee: self.performance_fee.to_basis_points().unwrap_or(0),
                supply_cap_tao: self.supply_cap_tao,
                supply_cap_tusdt: self.supply_cap_tusdt,
                borrow_cap_tao: self.borrow_cap_tao,
                borrow_cap_tusdt: self.borrow_cap_tusdt,
            }
        }
    }

    /// Lending pool storage: roles, market state, user positions, and risk parameters.
    #[ink(storage)]
    pub struct TusdtLendingPool {
        // ── Roles ──
        governance: AccountId,
        maintainer: AccountId,
        treasury: AccountId,
        platform: AccountId,
        paused: bool,
        /// Reentrancy guard.
        busy: bool,

        // ── External refs ──
        /// TUSDT/TAO price oracle (existing instance).
        oracle: TusdtOracleRef,
        /// Existing TUSDT token (underlying borrowable + supplied asset).
        tusdt: TusdtErc20Ref,
        /// Single staking hotkey for all alpha collateral.
        pool_hotkey: AccountId,

        // ── Markets ──
        /// Market state by id (0 = TAO, 1 = TUSDT, 2+ = alpha netuids).
        markets: Mapping<u8, MarketState>,
        /// Enumeration of all market ids for pagination.
        market_keys: StorageVec<u8>,
        /// lToken (tusdt-erc20 child) address per supply market (0, 1).
        ltoken_by_market: Mapping<u8, AccountId>,
        /// Approved netuid → alpha market id.
        netuid_to_market: Mapping<u16, u8>,
        /// Alpha market id → netuid.
        market_to_netuid: Mapping<u8, u16>,
        /// Next market id for alpha markets.
        next_alpha_market_id: u8,

        // ── Per-market static params ──
        market_params: Mapping<u8, InterestRateParams>,
        alpha_params: Mapping<u16, AlphaMarketParams>,
        pending_market_params: Mapping<u8, PendingInterestParamsUpdate>,
        pending_alpha_params: Mapping<u16, PendingAlphaParamsUpdate>,

        // ── Global params + timelock ──
        global_params: PoolGlobalParams,
        pending_global_params: Option<PendingGlobalParamsUpdate>,

        // ── Alpha custody accounting ──
        /// Total alpha principal per netuid (Σ user alpha_principal).
        netuid_total_collateral: Mapping<u16, Balance>,
        /// Per-netuid yield accumulator for alpha performance fee.
        /// Effective collateral = alpha_principal × yield_index.
        netuid_yield_index: Mapping<u16, Ratio>,

        // ── User positions ──
        /// Per-(market_id, user) position.
        positions: Mapping<(u8, AccountId), Position>,
        /// Position keys for paginated enumeration.
        position_keys: StorageVec<(u8, AccountId)>,
    }

    /// Default interest-rate parameters for the TAO supply/borrow market.
    /// base=0%, slope1=4%, slope2=96%, optimal=80%, reserve_factor=20%
    fn default_tao_interest_params() -> InterestRateParams {
        InterestRateParams {
            base_rate: Ratio::from_inner(0),
            slope1: Ratio::from_basis_points(400),
            slope2: Ratio::from_basis_points(9600),
            optimal_utilization: Ratio::from_basis_points(8000),
            reserve_factor: Ratio::from_basis_points(2000),
        }
    }

    /// Default interest-rate parameters for the TUSDT supply/borrow market.
    /// base=0%, slope1=3%, slope2=97%, optimal=80%, reserve_factor=20%
    fn default_tusdt_interest_params() -> InterestRateParams {
        InterestRateParams {
            base_rate: Ratio::from_inner(0),
            slope1: Ratio::from_basis_points(300),
            slope2: Ratio::from_basis_points(9700),
            optimal_utilization: Ratio::from_basis_points(8000),
            reserve_factor: Ratio::from_basis_points(2000),
        }
    }

    /// Default alpha market parameters.
    /// collateral_factor=50%, liquidation_threshold=60%, liquidation_bonus=5%, supply_cap=unlimited
    fn default_alpha_params() -> AlphaMarketParams {
        AlphaMarketParams {
            collateral_factor: Ratio::from_basis_points(5000),
            liquidation_threshold: Ratio::from_basis_points(6000),
            liquidation_bonus: Ratio::from_basis_points(500),
            supply_cap: 0, // unlimited
        }
    }

    /// Default global parameters.
    fn default_global_params() -> PoolGlobalParams {
        PoolGlobalParams {
            max_oracle_age_ms: 1_800_000,                    // 30 min
            close_factor: Ratio::from_basis_points(5000),    // 50%
            performance_fee: Ratio::from_basis_points(2500), // 25%
            supply_cap_tao: 0,
            supply_cap_tusdt: 0,
            borrow_cap_tao: 0,
            borrow_cap_tusdt: 0,
        }
    }

    /// Events emitted by the lending pool.
    #[ink(event)]
    pub struct LiquidDeposited {
        #[ink(topic)]
        pub market: u8,
        #[ink(topic)]
        pub user: AccountId,
        pub amount: Balance,
        pub ltoken_scaled: Balance,
    }

    #[ink(event)]
    pub struct LiquidWithdrawn {
        #[ink(topic)]
        pub market: u8,
        #[ink(topic)]
        pub user: AccountId,
        pub amount: Balance,
        pub ltoken_scaled: Balance,
    }

    #[ink(event)]
    pub struct Borrowed {
        #[ink(topic)]
        pub market: u8,
        #[ink(topic)]
        pub user: AccountId,
        pub amount: Balance,
    }

    #[ink(event)]
    pub struct Repaid {
        #[ink(topic)]
        pub market: u8,
        #[ink(topic)]
        pub user: AccountId,
        pub amount: Balance,
    }

    #[ink(event)]
    pub struct AlphaCollateralDeposited {
        #[ink(topic)]
        pub netuid: u16,
        #[ink(topic)]
        pub user: AccountId,
        pub amount: Balance,
        pub market_id: u8,
    }

    #[ink(event)]
    pub struct AlphaCollateralWithdrawn {
        #[ink(topic)]
        pub netuid: u16,
        #[ink(topic)]
        pub user: AccountId,
        pub amount: Balance,
        pub effective: Balance,
        #[ink(topic)]
        pub dest_coldkey: AccountId,
    }

    #[ink(event)]
    pub struct Liquidated {
        #[ink(topic)]
        pub user: AccountId,
        #[ink(topic)]
        pub collateral_netuid: u16,
        pub debt_market: u8,
        pub debt_covered: Balance,
        pub collateral_alpha: Balance,
        #[ink(topic)]
        pub liquidator: AccountId,
    }

    #[ink(event)]
    pub struct AlphaYieldClaimed {
        #[ink(topic)]
        pub netuid: u16,
        pub excess_alpha: Balance,
        pub performance_fee_alpha: Balance,
        pub tao_received: Balance,
        pub index_before: u128,
        pub index_after: u128,
    }

    #[ink(event)]
    pub struct ReserveClaimed {
        #[ink(topic)]
        pub market: u8,
        pub amount: Balance,
    }

    #[ink(event)]
    pub struct MarketAccrued {
        #[ink(topic)]
        pub market: u8,
        pub dt_hours: u64,
        pub utilization: u128,
        pub borrow_rate: u128,
        pub supply_rate: u128,
        pub reserve_delta: Balance,
    }

    #[ink(event)]
    pub struct MarketParamsUpdateScheduled {
        #[ink(topic)]
        pub market: u8,
        pub execute_after: u64,
    }

    #[ink(event)]
    pub struct MarketParamsUpdated {
        #[ink(topic)]
        pub market: u8,
    }

    #[ink(event)]
    pub struct MarketParamsUpdateCancelled {
        #[ink(topic)]
        pub market: u8,
    }

    #[ink(event)]
    pub struct AlphaParamsUpdateScheduled {
        #[ink(topic)]
        pub netuid: u16,
        pub execute_after: u64,
    }

    #[ink(event)]
    pub struct AlphaParamsUpdated {
        #[ink(topic)]
        pub netuid: u16,
    }

    #[ink(event)]
    pub struct AlphaParamsUpdateCancelled {
        #[ink(topic)]
        pub netuid: u16,
    }

    #[ink(event)]
    pub struct GlobalParamsUpdateScheduled {
        pub execute_after: u64,
    }

    #[ink(event)]
    pub struct GlobalParamsUpdated {}

    #[ink(event)]
    pub struct GlobalParamsUpdateCancelled {}

    #[ink(event)]
    pub struct NetuidApproved {
        #[ink(topic)]
        pub netuid: u16,
        pub approved: bool,
        pub market_id: Option<u8>,
    }

    #[ink(event)]
    pub struct LTokenAddressUpdated {
        #[ink(topic)]
        pub market: u8,
        pub old_ltoken: AccountId,
        pub new_ltoken: AccountId,
    }

    #[ink(event)]
    pub struct PoolGovernanceUpdated {
        pub previous: AccountId,
        pub new: AccountId,
    }

    #[ink(event)]
    pub struct PoolTreasuryUpdated {
        pub previous: AccountId,
        pub new: AccountId,
    }

    #[ink(event)]
    pub struct PoolPlatformUpdated {
        pub previous: AccountId,
        pub new: AccountId,
    }

    #[ink(event)]
    pub struct PoolHotkeyChanged {
        #[ink(topic)]
        pub old_hotkey: AccountId,
        #[ink(topic)]
        pub new_hotkey: AccountId,
    }

    #[ink(event)]
    pub struct PoolOracleAddressUpdated {
        pub previous: AccountId,
        pub new: AccountId,
    }

    #[ink(event)]
    pub struct TusdtAddressUpdated {
        pub previous: AccountId,
        pub new: AccountId,
    }

    #[ink(event)]
    pub struct PoolPaused {}

    #[ink(event)]
    pub struct PoolUnpaused {}

    #[ink(event)]
    pub struct PoolSurplusTusdtClaimed {
        pub recipient: AccountId,
        pub amount: Balance,
    }

    #[ink(event)]
    pub struct PoolNativeTransferredToTreasury {
        pub amount: Balance,
    }

    #[ink(event)]
    pub struct PoolMaintainerUpdated {
        #[ink(topic)]
        pub new_maintainer: AccountId,
    }

    /// Error types for the lending pool. All variants are fieldless for compact SCALE encoding.
    #[derive(Debug, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    pub enum Error {
        // ── Access ──
        NotGovernance,
        NotGovernanceOrPlatform,
        NotMaintainer,
        ContractPaused,
        Reentrancy,

        // ── Amounts & liquidity ──
        ZeroAmount,
        LiquidityInsufficient,
        MintBelowPrecision,
        InsufficientLTokenBalance,
        SupplyCapExceeded,
        BorrowCapExceeded,
        BorrowHealthExceeded,
        RepayAmountTooHigh,

        // ── Alpha collateral ──
        UnapprovedNetuid,
        NetuidHasPositions,
        InsufficientCollateral,
        InsufficientAvailableStake,
        StakeTransferFailed,
        ChainExtensionFailed,
        NoAlphaStakeFound,
        TooManyNetuids,

        // ── Pricing & health ──
        OracleCallFailed,
        OraclePriceUnavailable,
        OraclePriceStale,
        HealthFactorBelowThreshold,
        NotLiquidatable,
        InvalidCollateralNetuid,
        InvalidDebtMarket,
        CloseFactorExceeded,
        CollateralAwardExceedsPosition,

        // ── Params / timelock ──
        InvalidRatio,
        InvalidParam,
        NoPendingMarketParamsUpdate,
        NoPendingAlphaParamsUpdate,
        NoPendingGlobalParamsUpdate,
        ParamsUpdateTimelockActive,

        // ── Cross-contract ──
        TokenContractCallFailed,
        TokenTransferFromFailed,
        LTokenCallFailed,
        TransferFailed,

        // ── General ──
        MarketNotFound,
        PositionNotFound,
        ArithmeticError,
    }

    pub type Result<T> = core::result::Result<T, Error>;

    impl TusdtLendingPool {
        /// Creates a new lending pool.
        ///
        /// Spawns two lToken children (lTAO and lTUSDT) using the provided `ltoken_code_hash`
        /// (which should be the `tusdt-erc20` code hash). The deployer becomes the initial
        /// governance and platform.
        #[ink(constructor)]
        pub fn new(
            treasury: AccountId,
            tusdt_address: AccountId,
            oracle_address: AccountId,
            ltoken_code_hash: Hash,
            pool_hotkey: AccountId,
        ) -> Self {
            let caller = Self::env().caller();
            let contract_account = Self::env().account_id();
            let now = Self::env().block_timestamp();

            // Spawn lTAO (market 0) and lTUSDT (market 1) as child tusdt-erc20 instances.
            let ltao = TusdtErc20Ref::new(contract_account)
                .code_hash(ltoken_code_hash)
                .endowment(0)
                .salt_bytes([3u8; 32])
                .instantiate();
            let ltusdt = TusdtErc20Ref::new(contract_account)
                .code_hash(ltoken_code_hash)
                .endowment(0)
                .salt_bytes([4u8; 32])
                .instantiate();

            let mut markets = Mapping::default();
            markets.insert(0, &MarketState::new(now));
            markets.insert(1, &MarketState::new(now));

            let mut ltoken_by_market = Mapping::default();
            ltoken_by_market.insert(0, &ltao.to_account_id());
            ltoken_by_market.insert(1, &ltusdt.to_account_id());

            let mut market_keys = StorageVec::new();
            market_keys.push(&0);
            market_keys.push(&1);

            let mut market_params = Mapping::default();
            market_params.insert(0, &default_tao_interest_params());
            market_params.insert(1, &default_tusdt_interest_params());

            Self {
                governance: caller,
                maintainer: caller,
                treasury,
                platform: caller,
                paused: false,
                busy: false,
                oracle: TusdtOracleRef::from_account_id(oracle_address),
                tusdt: TusdtErc20Ref::from_account_id(tusdt_address),
                pool_hotkey,
                markets,
                market_keys,
                ltoken_by_market,
                netuid_to_market: Mapping::default(),
                market_to_netuid: Mapping::default(),
                next_alpha_market_id: 2,
                market_params,
                alpha_params: Mapping::default(),
                pending_market_params: Mapping::default(),
                pending_alpha_params: Mapping::default(),
                global_params: default_global_params(),
                pending_global_params: None,
                netuid_total_collateral: Mapping::default(),
                netuid_yield_index: Mapping::default(),
                positions: Mapping::default(),
                position_keys: StorageVec::new(),
            }
        }

        /// Upgrade constructor: deploys a new pool reusing existing lToken children.
        #[ink(constructor)]
        pub fn new_upgrade(
            treasury: AccountId,
            tusdt_address: AccountId,
            oracle_address: AccountId,
            ltao_address: AccountId,
            ltusdt_address: AccountId,
            pool_hotkey: AccountId,
        ) -> Self {
            let caller = Self::env().caller();
            let now = Self::env().block_timestamp();

            let mut markets = Mapping::default();
            markets.insert(0, &MarketState::new(now));
            markets.insert(1, &MarketState::new(now));

            let mut ltoken_by_market = Mapping::default();
            ltoken_by_market.insert(0, &ltao_address);
            ltoken_by_market.insert(1, &ltusdt_address);

            let mut market_keys = StorageVec::new();
            market_keys.push(&0);
            market_keys.push(&1);

            let mut market_params = Mapping::default();
            market_params.insert(0, &default_tao_interest_params());
            market_params.insert(1, &default_tusdt_interest_params());

            Self {
                governance: caller,
                maintainer: caller,
                treasury,
                platform: caller,
                paused: false,
                busy: false,
                oracle: TusdtOracleRef::from_account_id(oracle_address),
                tusdt: TusdtErc20Ref::from_account_id(tusdt_address),
                pool_hotkey,
                markets,
                market_keys,
                ltoken_by_market,
                netuid_to_market: Mapping::default(),
                market_to_netuid: Mapping::default(),
                next_alpha_market_id: 2,
                market_params,
                alpha_params: Mapping::default(),
                pending_market_params: Mapping::default(),
                pending_alpha_params: Mapping::default(),
                global_params: default_global_params(),
                pending_global_params: None,
                netuid_total_collateral: Mapping::default(),
                netuid_yield_index: Mapping::default(),
                positions: Mapping::default(),
                position_keys: StorageVec::new(),
            }
        }
    }

    // Ratio arithmetic helpers (Ratio doesn't impl checked_add/checked_sub directly)
    fn ratio_add(a: Ratio, b: Ratio) -> Option<Ratio> {
        Some(Ratio::from_inner(a.into_inner().checked_add(b.into_inner())?))
    }
    fn ratio_sub(a: Ratio, b: Ratio) -> Option<Ratio> {
        Some(Ratio::from_inner(a.into_inner().checked_sub(b.into_inner())?))
    }

    impl TusdtLendingPool {
        // ─────────────────────────────────────────────────────────────
        // Access control
        // ─────────────────────────────────────────────────────────────

        pub(crate) fn ensure_idle(&mut self) -> Result<()> {
            if self.busy {
                return Err(Error::Reentrancy);
            }
            self.busy = true;
            Ok(())
        }

        pub(crate) fn set_idle(&mut self) {
            self.busy = false;
        }

        // ─────────────────────────────────────────────────────────────
        // Position key lifecycle
        // ─────────────────────────────────────────────────────────────

        /// Remove all occurrences of a key from `position_keys` by scanning
        /// backward. On each match we swap the tail element into the match
        /// slot and pop, so the operation is O(n) but only fires when a
        /// position becomes all-zeros (rare).
        pub(crate) fn remove_position_key(&mut self, key: &(u8, AccountId)) {
            let mut i = self.position_keys.len();
            while i > 0 {
                i = i.saturating_sub(1);
                if self.position_keys.get(i) == Some(*key) {
                    let last = self.position_keys.len().saturating_sub(1);
                    if i != last {
                        if let Some(tail) = self.position_keys.get(last) {
                            self.position_keys.set(i, &tail);
                        }
                    }
                    self.position_keys.pop();
                }
            }
        }

        /// Maintains the `position_keys` invariant after any position mutation:
        /// `position_keys` holds exactly one entry per non-zero position.
        ///
        /// * If the position is now non-zero → ensure the key is present
        ///   (pushes if absent; self-heals legacy duplicates by not pushing
        ///   again when already present).
        /// * If the position is now all-zeros → remove **all** occurrences
        ///   of the key (self-heals legacy duplicates).
        pub(crate) fn update_position_key(&mut self, market_id: u8, user: AccountId) {
            let key = (market_id, user);
            let pos = self.positions.get(key).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            let is_zero =
                pos.ltoken_balance == 0 && pos.scaled_debt == 0 && pos.alpha_principal == 0;

            if is_zero {
                self.remove_position_key(&key);
            } else {
                // Ensure key is present exactly once — scan first so legacy
                // duplicates never cause a second push (self-healing).
                let mut present = false;
                for i in 0..self.position_keys.len() {
                    if self.position_keys.get(i) == Some(key) {
                        present = true;
                        break;
                    }
                }
                if !present {
                    self.position_keys.push(&key);
                }
            }
        }

        pub(crate) fn ensure_governance(&self) -> Result<()> {
            if self.env().caller() != self.governance {
                return Err(Error::NotGovernance);
            }
            Ok(())
        }

        pub(crate) fn ensure_governance_or_platform(&self) -> Result<()> {
            let caller = self.env().caller();
            if caller != self.governance && caller != self.platform {
                return Err(Error::NotGovernanceOrPlatform);
            }
            Ok(())
        }

        pub(crate) fn ensure_maintainer(&self) -> Result<()> {
            let caller = self.env().caller();
            if caller != self.maintainer && caller != self.governance {
                return Err(Error::NotMaintainer);
            }
            Ok(())
        }

        pub(crate) fn ensure_not_paused(&self) -> Result<()> {
            if self.paused {
                return Err(Error::ContractPaused);
            }
            Ok(())
        }

        pub(crate) fn ensure_approved_netuid(&self, netuid: u16) -> Result<()> {
            if self.netuid_to_market.get(netuid).is_none() {
                return Err(Error::UnapprovedNetuid);
            }
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // Interest rate math
        // ─────────────────────────────────────────────────────────────

        pub(crate) fn accrue_interest(&mut self, market_id: u8) -> Result<()> {
            if market_id >= 2 {
                return Ok(());
            }
            let now = self.env().block_timestamp();
            let mut state = self.markets.get(market_id).ok_or(Error::MarketNotFound)?;
            let dt_ms = now.checked_sub(state.last_update).ok_or(Error::ArithmeticError)?;
            let dt_hours = dt_ms / tusdt_primitives::MILLISECONDS_PER_HOUR;
            if dt_hours == 0 || state.total_debt == 0 {
                if dt_ms > 0 {
                    state.last_update = now;
                    self.markets.insert(market_id, &state);
                }
                return Ok(());
            }
            let cash = self.market_cash(market_id)?;
            let total_liquidity =
                state.total_debt.checked_add(cash).ok_or(Error::ArithmeticError)?;
            let utilization = if total_liquidity == 0 {
                Ratio::from_inner(0)
            } else {
                Ratio::from_integer(state.total_debt.into())
                    .checked_div_int(total_liquidity.into())
                    .ok_or(Error::ArithmeticError)?
            };
            let params = self.market_params.get(market_id).ok_or(Error::MarketNotFound)?;
            let borrow_rate_annual = Self::compute_borrow_rate(&params, utilization)?;
            let one = Ratio::one();
            let hours_per_year = Ratio::from_integer(tusdt_primitives::HOURS_PER_YEAR);
            let borrow_rate_hourly = borrow_rate_annual
                .checked_div_int(hours_per_year.into_inner())
                .ok_or(Error::ArithmeticError)?;
            let one_minus_rf =
                ratio_sub(one, params.reserve_factor).ok_or(Error::ArithmeticError)?;
            let supply_rate_annual = borrow_rate_annual
                .checked_mul(utilization)
                .and_then(|r| r.checked_mul(one_minus_rf))
                .ok_or(Error::ArithmeticError)?;
            let supply_rate_hourly = supply_rate_annual
                .checked_div_int(hours_per_year.into_inner())
                .ok_or(Error::ArithmeticError)?;
            let borrow_growth = ratio_add(one, borrow_rate_hourly)
                .and_then(|f| f.checked_pow(dt_hours.into()))
                .ok_or(Error::ArithmeticError)?;
            let supply_growth = ratio_add(one, supply_rate_hourly)
                .and_then(|f| f.checked_pow(dt_hours.into()))
                .ok_or(Error::ArithmeticError)?;
            let debt_before = state.total_debt;
            let new_debt = borrow_growth
                .checked_mul_value(debt_before.into())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;
            let debt_interest = new_debt.checked_sub(debt_before).ok_or(Error::ArithmeticError)?;
            let new_exchange_rate =
                state.exchange_rate.checked_mul(supply_growth).ok_or(Error::ArithmeticError)?;
            let supply_interest = ratio_sub(new_exchange_rate, state.exchange_rate)
                .and_then(|g| g.checked_mul_value(state.total_supplied.into()))
                .and_then(|v| Balance::try_from(v).ok())
                .unwrap_or(0);
            let reserve_delta = debt_interest.saturating_sub(supply_interest);
            state.total_debt = new_debt;
            state.borrow_index =
                state.borrow_index.checked_mul(borrow_growth).ok_or(Error::ArithmeticError)?;
            state.exchange_rate = new_exchange_rate;
            state.reserve_accrued =
                state.reserve_accrued.checked_add(reserve_delta).ok_or(Error::ArithmeticError)?;
            state.last_update = now;
            self.markets.insert(market_id, &state);
            self.env().emit_event(MarketAccrued {
                market: market_id,
                dt_hours,
                utilization: utilization.into_inner(),
                borrow_rate: borrow_rate_annual.into_inner(),
                supply_rate: supply_rate_annual.into_inner(),
                reserve_delta,
            });
            Ok(())
        }

        pub(crate) fn compute_borrow_rate(
            params: &InterestRateParams,
            utilization: Ratio,
        ) -> Result<Ratio> {
            if utilization.is_zero() {
                return Ok(params.base_rate);
            }
            let one = Ratio::one();
            // Fixed-point division helper: (num * 1e18) / denom.
            // Safe because both ≤ 1e18, so num * 1e18 ≤ 1e36 < u128::MAX (~3.4e38).
            fn div_ratio(num: Ratio, denom: Ratio) -> Option<Ratio> {
                let num_inner = num.into_inner();
                let denom_inner = denom.into_inner();
                Some(Ratio::from_inner(
                    num_inner.checked_mul(Ratio::one().into_inner())?.checked_div(denom_inner)?,
                ))
            }
            if utilization <= params.optimal_utilization {
                let fraction = div_ratio(utilization, params.optimal_utilization)
                    .ok_or(Error::ArithmeticError)?;
                let term = params.slope1.checked_mul(fraction).ok_or(Error::ArithmeticError)?;
                ratio_add(params.base_rate, term).ok_or(Error::ArithmeticError)
            } else {
                let range =
                    ratio_sub(one, params.optimal_utilization).ok_or(Error::ArithmeticError)?;
                let excess = ratio_sub(utilization, params.optimal_utilization)
                    .ok_or(Error::ArithmeticError)?;
                let fraction = div_ratio(excess, range).ok_or(Error::ArithmeticError)?;
                let term = params.slope2.checked_mul(fraction).ok_or(Error::ArithmeticError)?;
                ratio_add(params.base_rate, params.slope1)
                    .and_then(|r| ratio_add(r, term))
                    .ok_or(Error::ArithmeticError)
            }
        }

        pub(crate) fn market_cash(&self, market_id: u8) -> Result<Balance> {
            match market_id {
                0 => Ok(self.env().balance()),
                1 => Ok(self.tusdt.balance_of(self.env().account_id())),
                _ => Err(Error::MarketNotFound),
            }
        }

        // ─────────────────────────────────────────────────────────────
        // Risk math
        // ─────────────────────────────────────────────────────────────

        pub(crate) fn get_oracle_price(&self) -> Result<Ratio> {
            let price_data = self.oracle.get_latest_price().ok_or(Error::OraclePriceUnavailable)?;
            let now = self.env().block_timestamp();
            if price_data.committed_at > now {
                return Err(Error::OraclePriceUnavailable);
            }
            let age = now.checked_sub(price_data.committed_at).ok_or(Error::ArithmeticError)?;
            if age > self.global_params.max_oracle_age_ms {
                return Err(Error::OraclePriceStale);
            }
            if price_data.price.is_zero() {
                return Err(Error::OraclePriceUnavailable);
            }
            Ok(price_data.price)
        }

        pub(crate) fn collateral_price(&self, netuid: u16) -> Result<Ratio> {
            let tusdt_per_tao = self.get_oracle_price()?;
            let alpha_price_rao = self
                .env()
                .extension()
                .get_alpha_price(netuid)
                .map_err(|_| Error::ChainExtensionFailed)?;
            let alpha_to_tao = Self::alpha_price_rao_to_ratio(alpha_price_rao)?;
            tusdt_per_tao.checked_mul(alpha_to_tao).ok_or(Error::ArithmeticError)
        }

        pub(crate) fn alpha_price_rao_to_ratio(alpha_price_rao: u64) -> Result<Ratio> {
            Ratio::from_integer(alpha_price_rao.into())
                .checked_div_int(1_000_000_000u128)
                .ok_or(Error::ArithmeticError)
        }

        pub(crate) fn effective_alpha(&self, user: AccountId, netuid: u16) -> Result<Balance> {
            let market_id = self.netuid_to_market.get(netuid).ok_or(Error::UnapprovedNetuid)?;
            let pos = self.positions.get((market_id, user)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            if pos.alpha_principal == 0 {
                return Ok(0);
            }
            let yield_index = self.netuid_yield_index.get(netuid).unwrap_or(Ratio::one());
            yield_index
                .checked_mul_value(pos.alpha_principal.into())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)
        }

        pub fn get_collateral_value_tusdt(&self, user: AccountId) -> Result<Balance> {
            let mut total: u128 = 0;
            let count = self.market_keys.len();
            for i in 0..count {
                let market_id = self.market_keys.get(i).ok_or(Error::MarketNotFound)?;
                if market_id < 2 {
                    continue;
                }
                let netuid = self.market_to_netuid.get(market_id).ok_or(Error::MarketNotFound)?;
                let effective = self.effective_alpha(user, netuid)?;
                if effective == 0 {
                    continue;
                }
                let price = self.collateral_price(netuid)?;
                total = total
                    .checked_add(
                        price.checked_mul_value(effective.into()).ok_or(Error::ArithmeticError)?,
                    )
                    .ok_or(Error::ArithmeticError)?;
            }
            Balance::try_from(total).map_err(|_| Error::ArithmeticError)
        }

        pub fn get_debt_value_tusdt(&self, user: AccountId) -> Result<Balance> {
            let tusdt_state = self.markets.get(1).ok_or(Error::MarketNotFound)?;
            let tusdt_pos = self.positions.get((1, user)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            let tusdt_debt = if tusdt_pos.scaled_debt == 0 {
                0u128
            } else {
                tusdt_state
                    .borrow_index
                    .checked_mul_value(tusdt_pos.scaled_debt.into())
                    .ok_or(Error::ArithmeticError)?
            };
            let tao_state = self.markets.get(0).ok_or(Error::MarketNotFound)?;
            let tao_pos = self.positions.get((0, user)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            let tao_debt = if tao_pos.scaled_debt == 0 {
                0u128
            } else {
                tao_state
                    .borrow_index
                    .checked_mul_value(tao_pos.scaled_debt.into())
                    .ok_or(Error::ArithmeticError)?
            };
            let tao_debt_tusdt = if tao_debt > 0 {
                self.get_oracle_price()?
                    .checked_mul_value(tao_debt)
                    .ok_or(Error::ArithmeticError)?
            } else {
                0
            };
            Balance::try_from(tusdt_debt.checked_add(tao_debt_tusdt).ok_or(Error::ArithmeticError)?)
                .map_err(|_| Error::ArithmeticError)
        }

        pub fn get_health_factor(&self, user: AccountId) -> Result<Option<Ratio>> {
            let debt_value = self.get_debt_value_tusdt(user)?;
            if debt_value == 0 {
                return Ok(None);
            }
            let collateral_value = self.get_collateral_value_tusdt(user)?;
            if collateral_value == 0 {
                return Ok(Some(Ratio::from_inner(0)));
            }
            let threshold = self.max_liquidation_threshold_for_user(user)?;
            let health = threshold
                .checked_mul_value(collateral_value.into())
                .and_then(|v| Ratio::from_inner(v).checked_div_int(debt_value.into()))
                .ok_or(Error::ArithmeticError)?;
            Ok(Some(health))
        }

        fn max_liquidation_threshold_for_user(&self, user: AccountId) -> Result<Ratio> {
            let mut max_threshold = Ratio::from_inner(0);
            let count = self.market_keys.len();
            for i in 0..count {
                let market_id = self.market_keys.get(i).ok_or(Error::MarketNotFound)?;
                if market_id < 2 {
                    continue;
                }
                let netuid = self.market_to_netuid.get(market_id).ok_or(Error::MarketNotFound)?;
                let pos = self.positions.get((market_id, user)).unwrap_or(Position {
                    ltoken_balance: 0,
                    scaled_debt: 0,
                    alpha_principal: 0,
                });
                if pos.alpha_principal == 0 {
                    continue;
                }
                let params = self.alpha_params.get(netuid).unwrap_or(default_alpha_params());
                if params.liquidation_threshold.into_inner() > max_threshold.into_inner() {
                    max_threshold = params.liquidation_threshold;
                }
            }
            if max_threshold.is_zero() {
                return Ok(Ratio::from_basis_points(6000));
            }
            Ok(max_threshold)
        }

        pub fn get_available_borrow_tusdt(&self, user: AccountId) -> Result<Balance> {
            let collateral_value = self.get_collateral_value_tusdt(user)?;
            if collateral_value == 0 {
                return Ok(0);
            }
            let debt_value = self.get_debt_value_tusdt(user)?;
            let factor = self.min_collateral_factor_for_user(user)?;
            let max_borrow = factor
                .checked_mul_value(collateral_value.into())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;
            if max_borrow <= debt_value {
                return Ok(0);
            }
            max_borrow.checked_sub(debt_value).ok_or(Error::ArithmeticError)
        }

        fn min_collateral_factor_for_user(&self, user: AccountId) -> Result<Ratio> {
            let mut min_factor = Ratio::one();
            let count = self.market_keys.len();
            let mut found = false;
            for i in 0..count {
                let market_id = self.market_keys.get(i).ok_or(Error::MarketNotFound)?;
                if market_id < 2 {
                    continue;
                }
                let netuid = self.market_to_netuid.get(market_id).ok_or(Error::MarketNotFound)?;
                let pos = self.positions.get((market_id, user)).unwrap_or(Position {
                    ltoken_balance: 0,
                    scaled_debt: 0,
                    alpha_principal: 0,
                });
                if pos.alpha_principal == 0 {
                    continue;
                }
                found = true;
                let params = self.alpha_params.get(netuid).unwrap_or(default_alpha_params());
                if params.collateral_factor.into_inner() < min_factor.into_inner() {
                    min_factor = params.collateral_factor;
                }
            }
            if !found {
                return Ok(Ratio::from_inner(0));
            }
            Ok(min_factor)
        }

        pub fn is_liquidatable(&self, user: AccountId) -> Result<bool> {
            match self.get_health_factor(user)? {
                None => Ok(false),
                Some(hf) => Ok(hf.into_inner() < Ratio::one().into_inner()),
            }
        }

        // ─────────────────────────────────────────────────────────────
        // Param validation
        // ─────────────────────────────────────────────────────────────

        pub(crate) fn interest_params_from_config(
            config: InterestRateParamsConfig,
        ) -> Result<InterestRateParams> {
            if config.optimal_utilization == 0 || config.optimal_utilization > 10_000 {
                return Err(Error::InvalidParam);
            }
            let slope_total =
                config.slope1.checked_add(config.slope2).ok_or(Error::ArithmeticError)?;
            if slope_total > 10_000 {
                return Err(Error::InvalidParam);
            }
            if config.reserve_factor >= 10_000 {
                return Err(Error::InvalidParam);
            }
            Ok(InterestRateParams {
                base_rate: Ratio::from_basis_points(config.base_rate),
                slope1: Ratio::from_basis_points(config.slope1),
                slope2: Ratio::from_basis_points(config.slope2),
                optimal_utilization: Ratio::from_basis_points(config.optimal_utilization),
                reserve_factor: Ratio::from_basis_points(config.reserve_factor),
            })
        }

        pub(crate) fn validate_interest_params(params: &InterestRateParams) -> Result<()> {
            let max_rate = Ratio::from_inner(
                params
                    .base_rate
                    .into_inner()
                    .checked_add(params.slope1.into_inner())
                    .and_then(|v| v.checked_add(params.slope2.into_inner()))
                    .ok_or(Error::ArithmeticError)?,
            );
            if max_rate.into_inner() > Ratio::from_basis_points(10_000).into_inner() {
                return Err(Error::InvalidRatio);
            }
            if params.optimal_utilization.is_zero()
                || params.optimal_utilization.into_inner() > Ratio::one().into_inner()
            {
                return Err(Error::InvalidRatio);
            }
            if params.reserve_factor.into_inner() >= Ratio::one().into_inner() {
                return Err(Error::InvalidRatio);
            }
            Ok(())
        }

        pub(crate) fn alpha_params_from_config(
            config: AlphaMarketParamsConfig,
        ) -> Result<AlphaMarketParams> {
            if config.collateral_factor >= config.liquidation_threshold {
                return Err(Error::InvalidParam);
            }
            if config.liquidation_threshold > 10_000 {
                return Err(Error::InvalidParam);
            }
            if config.liquidation_bonus > 2_500 {
                return Err(Error::InvalidParam);
            }
            Ok(AlphaMarketParams {
                collateral_factor: Ratio::from_basis_points(config.collateral_factor),
                liquidation_threshold: Ratio::from_basis_points(config.liquidation_threshold),
                liquidation_bonus: Ratio::from_basis_points(config.liquidation_bonus),
                supply_cap: config.supply_cap,
            })
        }

        pub(crate) fn validate_alpha_params(params: &AlphaMarketParams) -> Result<()> {
            if params.collateral_factor.is_zero()
                || params.collateral_factor.into_inner()
                    >= params.liquidation_threshold.into_inner()
            {
                return Err(Error::InvalidRatio);
            }
            if params.liquidation_threshold.into_inner() > Ratio::one().into_inner() {
                return Err(Error::InvalidRatio);
            }
            if params.liquidation_bonus.into_inner() > Ratio::from_basis_points(2_500).into_inner()
            {
                return Err(Error::InvalidRatio);
            }
            Ok(())
        }

        pub(crate) fn global_params_from_config(
            config: PoolGlobalParamsConfig,
        ) -> Result<PoolGlobalParams> {
            if config.max_oracle_age_ms == 0 {
                return Err(Error::InvalidParam);
            }
            if config.close_factor == 0 || config.close_factor > 5_000 {
                return Err(Error::InvalidParam);
            }
            if config.performance_fee > 5_000 {
                return Err(Error::InvalidParam);
            }
            Ok(PoolGlobalParams {
                max_oracle_age_ms: config.max_oracle_age_ms,
                close_factor: Ratio::from_basis_points(config.close_factor),
                performance_fee: Ratio::from_basis_points(config.performance_fee),
                supply_cap_tao: config.supply_cap_tao,
                supply_cap_tusdt: config.supply_cap_tusdt,
                borrow_cap_tao: config.borrow_cap_tao,
                borrow_cap_tusdt: config.borrow_cap_tusdt,
            })
        }

        pub(crate) fn validate_global_params(params: &PoolGlobalParams) -> Result<()> {
            if params.max_oracle_age_ms == 0 {
                return Err(Error::InvalidRatio);
            }
            if params.close_factor.is_zero()
                || params.close_factor.into_inner() > Ratio::from_basis_points(5_000).into_inner()
            {
                return Err(Error::InvalidRatio);
            }
            if params.performance_fee.into_inner() > Ratio::from_basis_points(5_000).into_inner() {
                return Err(Error::InvalidRatio);
            }
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // User-facing messages: supply / withdraw
        // ─────────────────────────────────────────────────────────────

        /// Supplies TAO into the pool. Caller must send native TAO via `transferred_value`.
        /// Mints lTAO receipt tokens at the current exchange rate.
        #[ink(message, payable)]
        pub fn supply_tao(&mut self, amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;
            if amount == 0 {
                return Err(Error::ZeroAmount);
            }
            let received = self.env().transferred_value();
            if received < amount {
                return Err(Error::TransferFailed);
            }

            self.ensure_idle()?;
            // Guard is locked; every error path below MUST call set_idle() before returning.

            // Accrue interest before any state change
            self.accrue_interest(0).inspect_err(|_| {
                self.set_idle();
            })?;

            // Cap check
            if self.global_params.supply_cap_tao > 0 {
                let state = self.markets.get(0).ok_or(Error::MarketNotFound)?;
                let projected =
                    state.total_supplied.checked_add(amount).ok_or(Error::ArithmeticError)?;
                if projected > self.global_params.supply_cap_tao {
                    self.set_idle();
                    return Err(Error::SupplyCapExceeded);
                }
            }

            let state = self.markets.get(0).ok_or(Error::MarketNotFound)?;
            let ltoken_addr = self.ltoken_by_market.get(0).ok_or(Error::MarketNotFound)?;
            let mut ltoken = TusdtErc20Ref::from_account_id(ltoken_addr);

            // Compute lToken amount: amount * ltoken_total_supply / total_supplied
            let ltoken_supply = ltoken.total_supply();
            let ltoken_scaled = if ltoken_supply == 0 || state.total_supplied == 0 {
                amount
            } else {
                Ratio::from_integer(amount.into())
                    .checked_mul_value(ltoken_supply.into())
                    .and_then(|v| v.checked_div(state.total_supplied as u128))
                    .and_then(|v| u64::try_from(v).ok())
                    .unwrap_or(0)
            };
            if ltoken_scaled == 0 {
                self.set_idle();
                return Err(Error::MintBelowPrecision);
            }

            // Effects: update market state and position
            let caller = self.env().caller();
            let mut state = self.markets.get(0).ok_or(Error::MarketNotFound)?;
            state.total_supplied =
                state.total_supplied.checked_add(amount).ok_or(Error::ArithmeticError)?;
            self.markets.insert(0, &state);

            let mut pos = self.positions.get((0, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            pos.ltoken_balance =
                pos.ltoken_balance.checked_add(ltoken_scaled).ok_or(Error::ArithmeticError)?;
            self.positions.insert((0, caller), &pos);
            self.update_position_key(0, caller);

            // Mint lTokens
            ltoken.mint(caller, ltoken_scaled).map_err(|_| Error::LTokenCallFailed)?;

            // Refund excess native transfer
            if received > amount {
                let excess = received.checked_sub(amount).ok_or(Error::ArithmeticError)?;
                self.env().transfer(caller, excess).map_err(|_| Error::TransferFailed)?;
            }

            self.env().emit_event(LiquidDeposited {
                market: 0,
                user: caller,
                amount,
                ltoken_scaled,
            });

            self.set_idle();
            Ok(())
        }

        /// Supplies TUSDT into the pool. Pulls TUSDT from caller via `transfer_from`.
        /// Mints lTUSDT receipt tokens at the current exchange rate.
        #[ink(message)]
        pub fn supply_tusdt(&mut self, amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;
            if amount == 0 {
                return Err(Error::ZeroAmount);
            }

            self.ensure_idle()?;
            self.accrue_interest(1).inspect_err(|_| {
                self.set_idle();
            })?;

            // Cap check
            if self.global_params.supply_cap_tusdt > 0 {
                let state = self.markets.get(1).ok_or(Error::MarketNotFound)?;
                let projected =
                    state.total_supplied.checked_add(amount).ok_or(Error::ArithmeticError)?;
                if projected > self.global_params.supply_cap_tusdt {
                    self.set_idle();
                    return Err(Error::SupplyCapExceeded);
                }
            }

            let caller = self.env().caller();
            let pool_addr = self.env().account_id();

            // Pull TUSDT from caller
            self.tusdt
                .transfer_from(caller, pool_addr, amount)
                .map_err(|_| Error::TokenTransferFromFailed)?;

            let state = self.markets.get(1).ok_or(Error::MarketNotFound)?;
            let ltoken_addr = self.ltoken_by_market.get(1).ok_or(Error::MarketNotFound)?;
            let mut ltoken = TusdtErc20Ref::from_account_id(ltoken_addr);

            // Compute lToken amount
            let ltoken_supply = ltoken.total_supply();
            let ltoken_scaled = if ltoken_supply == 0 || state.total_supplied == 0 {
                amount
            } else {
                Ratio::from_integer(amount.into())
                    .checked_mul_value(ltoken_supply.into())
                    .and_then(|v| v.checked_div(state.total_supplied as u128))
                    .and_then(|v| u64::try_from(v).ok())
                    .unwrap_or(0)
            };
            if ltoken_scaled == 0 {
                self.set_idle();
                return Err(Error::MintBelowPrecision);
            }

            // Effects
            let mut state = self.markets.get(1).ok_or(Error::MarketNotFound)?;
            state.total_supplied =
                state.total_supplied.checked_add(amount).ok_or(Error::ArithmeticError)?;
            self.markets.insert(1, &state);

            let mut pos = self.positions.get((1, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            pos.ltoken_balance =
                pos.ltoken_balance.checked_add(ltoken_scaled).ok_or(Error::ArithmeticError)?;
            self.positions.insert((1, caller), &pos);
            self.update_position_key(1, caller);

            // Mint lTokens
            ltoken.mint(caller, ltoken_scaled).map_err(|_| Error::LTokenCallFailed)?;

            self.env().emit_event(LiquidDeposited {
                market: 1,
                user: caller,
                amount,
                ltoken_scaled,
            });

            self.set_idle();
            Ok(())
        }

        /// Withdraws TAO by burning lTAO tokens. Transfers native TAO to caller.
        #[ink(message)]
        pub fn withdraw_tao(&mut self, ltoken_amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;
            if ltoken_amount == 0 {
                return Err(Error::ZeroAmount);
            }

            self.ensure_idle()?;

            self.accrue_interest(0).inspect_err(|_| {
                self.set_idle();
            })?;

            let caller = self.env().caller();
            let state = self.markets.get(0).ok_or(Error::MarketNotFound)?;
            let ltoken_addr = self.ltoken_by_market.get(0).ok_or(Error::MarketNotFound)?;
            let mut ltoken = TusdtErc20Ref::from_account_id(ltoken_addr);

            // Check position
            let pos = self.positions.get((0, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            if pos.ltoken_balance < ltoken_amount {
                self.set_idle();
                return Err(Error::InsufficientLTokenBalance);
            }

            // Compute underlying amount
            let ltoken_supply = ltoken.total_supply();
            let underlying = if ltoken_supply == 0 || state.total_supplied == 0 {
                0
            } else {
                Ratio::from_integer(ltoken_amount.into())
                    .checked_mul_value(state.total_supplied.into())
                    .and_then(|v| v.checked_div(ltoken_supply as u128))
                    .and_then(|v| u64::try_from(v).ok())
                    .unwrap_or(0)
            };
            if underlying == 0 {
                self.set_idle();
                return Err(Error::MintBelowPrecision);
            }

            // Liquidity check
            let cash = self.market_cash(0)?;
            if underlying > cash {
                self.set_idle();
                return Err(Error::LiquidityInsufficient);
            }

            // Note: TAO supply is NOT collateral, so no health check is needed here.
            // Only alpha collateral positions affect borrow health.

            // Effects: burn lTokens, update market state
            ltoken.burn(caller, ltoken_amount).map_err(|_| Error::LTokenCallFailed)?;

            let mut state = self.markets.get(0).ok_or(Error::MarketNotFound)?;
            state.total_supplied =
                state.total_supplied.checked_sub(underlying).ok_or(Error::ArithmeticError)?;
            self.markets.insert(0, &state);

            let mut pos = self.positions.get((0, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            pos.ltoken_balance =
                pos.ltoken_balance.checked_sub(ltoken_amount).ok_or(Error::ArithmeticError)?;
            self.positions.insert((0, caller), &pos);
            self.update_position_key(0, caller);

            // Transfer TAO
            self.env().transfer(caller, underlying).map_err(|_| Error::TransferFailed)?;

            self.env().emit_event(LiquidWithdrawn {
                market: 0,
                user: caller,
                amount: underlying,
                ltoken_scaled: ltoken_amount,
            });

            self.set_idle();
            Ok(())
        }

        /// Withdraws TUSDT by burning lTUSDT tokens. Transfers TUSDT to caller.
        #[ink(message)]
        pub fn withdraw_tusdt(&mut self, ltoken_amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;
            if ltoken_amount == 0 {
                return Err(Error::ZeroAmount);
            }

            self.ensure_idle()?;
            self.accrue_interest(1).inspect_err(|_| {
                self.set_idle();
            })?;

            let caller = self.env().caller();
            let state = self.markets.get(1).ok_or(Error::MarketNotFound)?;
            let ltoken_addr = self.ltoken_by_market.get(1).ok_or(Error::MarketNotFound)?;
            let mut ltoken = TusdtErc20Ref::from_account_id(ltoken_addr);

            let pos = self.positions.get((1, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            if pos.ltoken_balance < ltoken_amount {
                self.set_idle();
                return Err(Error::InsufficientLTokenBalance);
            }

            let ltoken_supply = ltoken.total_supply();
            let underlying = if ltoken_supply == 0 || state.total_supplied == 0 {
                0
            } else {
                Ratio::from_integer(ltoken_amount.into())
                    .checked_mul_value(state.total_supplied.into())
                    .and_then(|v| v.checked_div(ltoken_supply as u128))
                    .and_then(|v| u64::try_from(v).ok())
                    .unwrap_or(0)
            };
            if underlying == 0 {
                self.set_idle();
                return Err(Error::MintBelowPrecision);
            }

            let cash = self.market_cash(1)?;
            if underlying > cash {
                self.set_idle();
                return Err(Error::LiquidityInsufficient);
            }

            // Effects
            ltoken.burn(caller, ltoken_amount).map_err(|_| Error::LTokenCallFailed)?;

            let mut state = self.markets.get(1).ok_or(Error::MarketNotFound)?;
            state.total_supplied =
                state.total_supplied.checked_sub(underlying).ok_or(Error::ArithmeticError)?;
            self.markets.insert(1, &state);

            let mut pos = self.positions.get((1, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            pos.ltoken_balance =
                pos.ltoken_balance.checked_sub(ltoken_amount).ok_or(Error::ArithmeticError)?;
            self.positions.insert((1, caller), &pos);
            self.update_position_key(1, caller);

            self.tusdt.transfer(caller, underlying).map_err(|_| Error::TokenContractCallFailed)?;

            self.env().emit_event(LiquidWithdrawn {
                market: 1,
                user: caller,
                amount: underlying,
                ltoken_scaled: ltoken_amount,
            });

            self.set_idle();
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // User-facing messages: borrow / repay
        // ─────────────────────────────────────────────────────────────

        /// Borrows TAO against the caller's alpha collateral.
        #[ink(message)]
        pub fn borrow_tao(&mut self, amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;
            if amount == 0 {
                return Err(Error::ZeroAmount);
            }

            self.ensure_idle()?;
            self.accrue_interest(0).inspect_err(|_| {
                self.set_idle();
            })?;

            let caller = self.env().caller();

            // Cap check
            if self.global_params.borrow_cap_tao > 0 {
                let state = self.markets.get(0).ok_or(Error::MarketNotFound)?;
                let projected =
                    state.total_debt.checked_add(amount).ok_or(Error::ArithmeticError)?;
                if projected > self.global_params.borrow_cap_tao {
                    self.set_idle();
                    return Err(Error::BorrowCapExceeded);
                }
            }

            // Liquidity check
            let cash = self.market_cash(0)?;
            if amount > cash {
                self.set_idle();
                return Err(Error::LiquidityInsufficient);
            }

            // Health check: convert borrow amount to TUSDT equivalent
            let tusdt_per_tao = self.get_oracle_price().inspect_err(|_| {
                self.set_idle();
            })?;
            let borrow_value_tusdt = tusdt_per_tao
                .checked_mul_value(amount.into())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;

            let available = self.get_available_borrow_tusdt(caller)?;
            if borrow_value_tusdt > available {
                self.set_idle();
                return Err(Error::BorrowHealthExceeded);
            }

            // Effects
            let state = self.markets.get(0).ok_or(Error::MarketNotFound)?;
            let scaled = Ratio::from_integer(amount.into())
                .checked_div_value(state.borrow_index.into_inner())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;

            let mut state = state;
            state.total_debt =
                state.total_debt.checked_add(amount).ok_or(Error::ArithmeticError)?;
            self.markets.insert(0, &state);

            let mut pos = self.positions.get((0, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            pos.scaled_debt = pos.scaled_debt.checked_add(scaled).ok_or(Error::ArithmeticError)?;
            self.positions.insert((0, caller), &pos);
            self.update_position_key(0, caller);

            // Transfer TAO to borrower
            self.env().transfer(caller, amount).map_err(|_| Error::TransferFailed)?;

            self.env().emit_event(Borrowed { market: 0, user: caller, amount });

            self.set_idle();
            Ok(())
        }

        /// Borrows TUSDT against the caller's alpha collateral.
        #[ink(message)]
        pub fn borrow_tusdt(&mut self, amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;
            if amount == 0 {
                return Err(Error::ZeroAmount);
            }

            self.ensure_idle()?;
            self.accrue_interest(1).inspect_err(|_| {
                self.set_idle();
            })?;

            let caller = self.env().caller();

            // Cap check
            if self.global_params.borrow_cap_tusdt > 0 {
                let state = self.markets.get(1).ok_or(Error::MarketNotFound)?;
                let projected =
                    state.total_debt.checked_add(amount).ok_or(Error::ArithmeticError)?;
                if projected > self.global_params.borrow_cap_tusdt {
                    self.set_idle();
                    return Err(Error::BorrowCapExceeded);
                }
            }

            // Liquidity check
            let cash = self.market_cash(1)?;
            if amount > cash {
                self.set_idle();
                return Err(Error::LiquidityInsufficient);
            }

            // Health check
            let available = self.get_available_borrow_tusdt(caller)?;
            if amount > available {
                self.set_idle();
                return Err(Error::BorrowHealthExceeded);
            }

            // Effects
            let state = self.markets.get(1).ok_or(Error::MarketNotFound)?;
            let scaled = Ratio::from_integer(amount.into())
                .checked_div_value(state.borrow_index.into_inner())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;

            let mut state = state;
            state.total_debt =
                state.total_debt.checked_add(amount).ok_or(Error::ArithmeticError)?;
            self.markets.insert(1, &state);

            let mut pos = self.positions.get((1, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            pos.scaled_debt = pos.scaled_debt.checked_add(scaled).ok_or(Error::ArithmeticError)?;
            self.positions.insert((1, caller), &pos);
            self.update_position_key(1, caller);

            // Transfer TUSDT to borrower
            self.tusdt.transfer(caller, amount).map_err(|_| Error::TokenContractCallFailed)?;

            self.env().emit_event(Borrowed { market: 1, user: caller, amount });

            self.set_idle();
            Ok(())
        }

        /// Repays TAO debt. Caller sends native TAO via `transferred_value`.
        #[ink(message, payable)]
        pub fn repay_tao(&mut self, amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;

            self.ensure_idle()?;
            self.accrue_interest(0).inspect_err(|_| {
                self.set_idle();
            })?;

            let caller = self.env().caller();
            let state = self.markets.get(0).ok_or(Error::MarketNotFound)?;

            // Compute actual debt
            let pos = self.positions.get((0, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            let debt = if pos.scaled_debt == 0 {
                0
            } else {
                state
                    .borrow_index
                    .checked_mul_value(pos.scaled_debt.into())
                    .and_then(|v| Balance::try_from(v).ok())
                    .unwrap_or(0)
            };

            let repay_amount = min(amount, debt);
            if repay_amount == 0 {
                self.set_idle();
                return Ok(());
            }

            // Verify payment
            let received = self.env().transferred_value();
            if received < repay_amount {
                self.set_idle();
                return Err(Error::TransferFailed);
            }
            if received > repay_amount {
                let excess = received.checked_sub(repay_amount).ok_or(Error::ArithmeticError)?;
                self.env().transfer(caller, excess).map_err(|_| Error::TransferFailed)?;
            }

            // Effects
            let scaled_repaid = Ratio::from_integer(repay_amount.into())
                .checked_div_value(state.borrow_index.into_inner())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;

            let mut state = state;
            state.total_debt =
                state.total_debt.checked_sub(repay_amount).ok_or(Error::ArithmeticError)?;
            self.markets.insert(0, &state);

            let mut pos = pos;
            pos.scaled_debt =
                pos.scaled_debt.checked_sub(scaled_repaid).ok_or(Error::ArithmeticError)?;
            self.positions.insert((0, caller), &pos);
            self.update_position_key(0, caller);

            // Note: repaid TAO stays in pool as cash — no burn.

            self.env().emit_event(Repaid { market: 0, user: caller, amount: repay_amount });

            self.set_idle();
            Ok(())
        }

        /// Repays TUSDT debt. Pulls TUSDT from caller via transfer_from.
        #[ink(message)]
        pub fn repay_tusdt(&mut self, amount: Balance) -> Result<()> {
            self.ensure_not_paused()?;

            self.ensure_idle()?;
            self.accrue_interest(1).inspect_err(|_| {
                self.set_idle();
            })?;

            let caller = self.env().caller();
            let pool_addr = self.env().account_id();
            let state = self.markets.get(1).ok_or(Error::MarketNotFound)?;

            let pos = self.positions.get((1, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            let debt = if pos.scaled_debt == 0 {
                0
            } else {
                state
                    .borrow_index
                    .checked_mul_value(pos.scaled_debt.into())
                    .and_then(|v| Balance::try_from(v).ok())
                    .unwrap_or(0)
            };

            let repay_amount = min(amount, debt);
            if repay_amount == 0 {
                self.set_idle();
                return Ok(());
            }

            // Pull TUSDT from caller
            self.tusdt
                .transfer_from(caller, pool_addr, repay_amount)
                .map_err(|_| Error::TokenTransferFromFailed)?;

            // Effects
            let scaled_repaid = Ratio::from_integer(repay_amount.into())
                .checked_div_value(state.borrow_index.into_inner())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;

            let mut state = state;
            state.total_debt =
                state.total_debt.checked_sub(repay_amount).ok_or(Error::ArithmeticError)?;
            self.markets.insert(1, &state);

            let mut pos = pos;
            pos.scaled_debt =
                pos.scaled_debt.checked_sub(scaled_repaid).ok_or(Error::ArithmeticError)?;
            self.positions.insert((1, caller), &pos);
            self.update_position_key(1, caller);

            self.env().emit_event(Repaid { market: 1, user: caller, amount: repay_amount });

            self.set_idle();
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // Alpha collateral: deposit / withdraw
        // ─────────────────────────────────────────────────────────────

        /// Deposits alpha stake as collateral. Uses chain extension func 25 to atomically
        /// pull the caller's stake from under `pool_hotkey` to the pool contract.
        #[ink(message)]
        pub fn deposit_alpha(&mut self, netuid: u16, amount: Balance) -> Result<u8> {
            // All validation checks FIRST (before reentrancy guard and chain extension)
            self.ensure_not_paused()?;
            if amount == 0 {
                return Err(Error::ZeroAmount);
            }
            self.ensure_approved_netuid(netuid)?;

            // Supply cap check BEFORE the chain extension call (CEI: check, then external call, then effects)
            let params = self.alpha_params.get(netuid).unwrap_or(default_alpha_params());
            if params.supply_cap > 0 {
                let current = self.netuid_total_collateral.get(netuid).unwrap_or_default();
                let projected = current.checked_add(amount).ok_or(Error::ArithmeticError)?;
                if projected > params.supply_cap {
                    return Err(Error::SupplyCapExceeded);
                }
            }

            self.ensure_idle()?;

            // Atomic pull via caller_transfer_stake (func 25)
            self.env()
                .extension()
                .caller_transfer_stake(
                    self.env().account_id(),
                    self.pool_hotkey,
                    netuid,
                    netuid,
                    amount,
                )
                .map_err(|_| {
                    self.set_idle();
                    Error::StakeTransferFailed
                })?;

            let caller = self.env().caller();
            let market_id = self.netuid_to_market.get(netuid).ok_or(Error::UnapprovedNetuid)?;

            // Effects
            let mut pos = self.positions.get((market_id, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            pos.alpha_principal =
                pos.alpha_principal.checked_add(amount).ok_or(Error::ArithmeticError)?;
            self.positions.insert((market_id, caller), &pos);
            self.update_position_key(market_id, caller);

            let current = self.netuid_total_collateral.get(netuid).unwrap_or_default();
            self.netuid_total_collateral
                .insert(netuid, &current.checked_add(amount).ok_or(Error::ArithmeticError)?);

            self.env().emit_event(AlphaCollateralDeposited {
                netuid,
                user: caller,
                amount,
                market_id,
            });
            self.set_idle();
            Ok(market_id)
        }

        /// Withdraws alpha collateral. Only allowed if the user remains healthy after withdrawal.
        #[ink(message)]
        pub fn withdraw_alpha(
            &mut self,
            netuid: u16,
            amount: Balance,
            dest_coldkey: AccountId,
        ) -> Result<()> {
            self.ensure_not_paused()?;
            if amount == 0 {
                return Err(Error::ZeroAmount);
            }

            self.ensure_idle()?;

            let caller = self.env().caller();
            let market_id = self.netuid_to_market.get(netuid).ok_or(Error::UnapprovedNetuid)?;

            let pos = self.positions.get((market_id, caller)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            if pos.alpha_principal < amount {
                self.set_idle();
                return Err(Error::InsufficientCollateral);
            }

            // Compute effective amount (with yield index)
            let yield_index = self.netuid_yield_index.get(netuid).unwrap_or(Ratio::one());
            let effective = yield_index
                .checked_mul_value(amount.into())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;

            // Check stake availability
            let availability = self
                .env()
                .extension()
                .get_stake_availability(self.env().account_id(), netuid)
                .map_err(|_| Error::ChainExtensionFailed)?;
            if effective > availability.available {
                self.set_idle();
                return Err(Error::InsufficientAvailableStake);
            }

            // Health check: ensure position stays healthy after withdrawal
            let has_debt = {
                let tao_pos = self.positions.get((0, caller)).unwrap_or(Position {
                    ltoken_balance: 0,
                    scaled_debt: 0,
                    alpha_principal: 0,
                });
                let tusdt_pos = self.positions.get((1, caller)).unwrap_or(Position {
                    ltoken_balance: 0,
                    scaled_debt: 0,
                    alpha_principal: 0,
                });
                tao_pos.scaled_debt > 0 || tusdt_pos.scaled_debt > 0
            };

            if has_debt {
                // Compute what health would be after withdrawal
                let mut new_pos = pos;
                new_pos.alpha_principal =
                    pos.alpha_principal.checked_sub(amount).ok_or(Error::ArithmeticError)?;
                self.positions.insert((market_id, caller), &new_pos);

                let current = self.netuid_total_collateral.get(netuid).unwrap_or_default();
                self.netuid_total_collateral
                    .insert(netuid, &current.checked_sub(amount).ok_or(Error::ArithmeticError)?);

                let health = self.get_health_factor(caller)?;
                // Revert the temporary state change
                self.positions.insert((market_id, caller), &pos);
                self.netuid_total_collateral.insert(netuid, &current);

                match health {
                    Some(hf) if hf.into_inner() < Ratio::one().into_inner() => {
                        self.set_idle();
                        return Err(Error::HealthFactorBelowThreshold);
                    },
                    _ => {},
                }
            }

            // Effects
            let mut pos = pos;
            pos.alpha_principal =
                pos.alpha_principal.checked_sub(amount).ok_or(Error::ArithmeticError)?;
            self.positions.insert((market_id, caller), &pos);
            self.update_position_key(market_id, caller);

            let current = self.netuid_total_collateral.get(netuid).unwrap_or_default();
            self.netuid_total_collateral
                .insert(netuid, &current.checked_sub(amount).ok_or(Error::ArithmeticError)?);

            // Transfer stake to destination coldkey (func 6)
            self.env()
                .extension()
                .transfer_stake(dest_coldkey, self.pool_hotkey, netuid, netuid, effective)
                .map_err(|_| {
                    // Revert position state on failure
                    let mut reverted = self.positions.get((market_id, caller)).unwrap_or(pos);
                    reverted.alpha_principal = reverted
                        .alpha_principal
                        .checked_add(amount)
                        .unwrap_or(reverted.alpha_principal);
                    self.positions.insert((market_id, caller), &reverted);
                    self.netuid_total_collateral.insert(netuid, &current);
                    Error::ChainExtensionFailed
                })?;

            self.env().emit_event(AlphaCollateralWithdrawn {
                netuid,
                user: caller,
                amount,
                effective,
                dest_coldkey,
            });

            self.set_idle();
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // Liquidation
        // ─────────────────────────────────────────────────────────────

        /// Liquidates an underwater position. The liquidator repays debt on behalf of the
        /// borrower and receives alpha collateral at a discount (liquidation bonus).
        #[ink(message, payable)]
        pub fn liquidate(
            &mut self,
            borrower: AccountId,
            debt_market: u8,
            debt_to_cover: Balance,
            collateral_netuid: u16,
        ) -> Result<()> {
            self.ensure_not_paused()?;
            self.ensure_idle()?;

            if debt_market > 1 {
                self.set_idle();
                return Err(Error::InvalidDebtMarket);
            }
            if debt_to_cover == 0 {
                self.set_idle();
                return Err(Error::ZeroAmount);
            }

            self.ensure_approved_netuid(collateral_netuid).inspect_err(|_| {
                self.set_idle();
            })?;

            // Accrue interest on the debt market
            self.accrue_interest(debt_market).inspect_err(|_| {
                self.set_idle();
            })?;

            let liquidator = self.env().caller();

            // Verify borrower is liquidatable
            if !self.is_liquidatable(borrower)? {
                self.set_idle();
                return Err(Error::NotLiquidatable);
            }

            // Get pricing
            let tusdt_per_tao = self.get_oracle_price().inspect_err(|_| {
                self.set_idle();
            })?;
            let collateral_price = self.collateral_price(collateral_netuid).inspect_err(|_| {
                self.set_idle();
            })?;

            // Compute borrower's total debt value and apply close factor
            let debt_value_tusdt = self.get_debt_value_tusdt(borrower)?;
            let max_cover_tusdt = self
                .global_params
                .close_factor
                .checked_mul_value(debt_value_tusdt.into())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;

            // Convert debt_to_cover to TUSDT equivalent
            let cover_tusdt = if debt_market == 0 {
                tusdt_per_tao
                    .checked_mul_value(debt_to_cover.into())
                    .and_then(|v| Balance::try_from(v).ok())
                    .ok_or(Error::ArithmeticError)?
            } else {
                debt_to_cover
            };
            let cover_tusdt = min(cover_tusdt, max_cover_tusdt);
            if cover_tusdt == 0 {
                self.set_idle();
                return Err(Error::ZeroAmount);
            }

            // Re-compute actual debt units to cover (cap at borrower's market debt)
            let state = self.markets.get(debt_market).ok_or(Error::MarketNotFound)?;
            let pos = self.positions.get((debt_market, borrower)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            let borrower_debt = if pos.scaled_debt == 0 {
                0
            } else {
                state
                    .borrow_index
                    .checked_mul_value(pos.scaled_debt.into())
                    .and_then(|v| Balance::try_from(v).ok())
                    .unwrap_or(0)
            };

            let actual_debt_units = if debt_market == 0 {
                let cover_tao = tusdt_per_tao
                    .checked_div_value(cover_tusdt.into())
                    .and_then(|v| Balance::try_from(v).ok())
                    .ok_or(Error::ArithmeticError)?;
                min(cover_tao, borrower_debt)
            } else {
                min(cover_tusdt, borrower_debt)
            };
            if actual_debt_units == 0 {
                self.set_idle();
                return Err(Error::ZeroAmount);
            }

            // Compute liquidator payment (in TUSDT value)
            let cover_value_tusdt = if debt_market == 0 {
                tusdt_per_tao
                    .checked_mul_value(actual_debt_units.into())
                    .and_then(|v| Balance::try_from(v).ok())
                    .ok_or(Error::ArithmeticError)?
            } else {
                actual_debt_units
            };

            // Compute collateral to seize with bonus
            let alpha_params =
                self.alpha_params.get(collateral_netuid).unwrap_or(default_alpha_params());
            let bonus_multiplier = Ratio::from_inner(
                Ratio::one()
                    .into_inner()
                    .checked_add(alpha_params.liquidation_bonus.into_inner())
                    .ok_or(Error::ArithmeticError)?,
            );
            let collateral_value_tusdt = bonus_multiplier
                .checked_mul_value(cover_value_tusdt.into())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;
            let alpha_to_seize = Ratio::from_integer(collateral_value_tusdt.into())
                .checked_div_value(collateral_price.into_inner())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;

            // Apply yield index to get principal
            let yield_index =
                self.netuid_yield_index.get(collateral_netuid).unwrap_or(Ratio::one());
            let alpha_principal_to_seize = yield_index
                .checked_div_value(alpha_to_seize.into())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;

            let collateral_market_id =
                self.netuid_to_market.get(collateral_netuid).ok_or(Error::UnapprovedNetuid)?;
            let collateral_pos = self
                .positions
                .get((collateral_market_id, borrower))
                .unwrap_or(Position { ltoken_balance: 0, scaled_debt: 0, alpha_principal: 0 });
            if alpha_principal_to_seize > collateral_pos.alpha_principal {
                self.set_idle();
                return Err(Error::CollateralAwardExceedsPosition);
            }

            // ── Effects (all before external calls) ──

            // 1. Reduce borrower debt
            let scaled_repaid = Ratio::from_integer(actual_debt_units.into())
                .checked_div_value(state.borrow_index.into_inner())
                .and_then(|v| Balance::try_from(v).ok())
                .ok_or(Error::ArithmeticError)?;

            let mut state = state;
            state.total_debt =
                state.total_debt.checked_sub(actual_debt_units).ok_or(Error::ArithmeticError)?;
            self.markets.insert(debt_market, &state);

            let mut pos = pos;
            pos.scaled_debt =
                pos.scaled_debt.checked_sub(scaled_repaid).ok_or(Error::ArithmeticError)?;
            self.positions.insert((debt_market, borrower), &pos);
            self.update_position_key(debt_market, borrower);

            // 2. Reduce borrower alpha collateral
            let mut collateral_pos = collateral_pos;
            collateral_pos.alpha_principal = collateral_pos
                .alpha_principal
                .checked_sub(alpha_principal_to_seize)
                .ok_or(Error::ArithmeticError)?;
            self.positions.insert((collateral_market_id, borrower), &collateral_pos);
            self.update_position_key(collateral_market_id, borrower);

            let netuid_collateral =
                self.netuid_total_collateral.get(collateral_netuid).unwrap_or_default();
            self.netuid_total_collateral.insert(
                collateral_netuid,
                &netuid_collateral
                    .checked_sub(alpha_principal_to_seize)
                    .ok_or(Error::ArithmeticError)?,
            );

            // ── External calls ──

            // Accept liquidator's debt repayment
            if debt_market == 0 {
                let received = self.env().transferred_value();
                if received < actual_debt_units {
                    // Revert would happen automatically via panic, but we mark state inconsistent
                    // The CEI ordering means the whole tx reverts on failure.
                    return Err(Error::TransferFailed);
                }
                if received > actual_debt_units {
                    let excess =
                        received.checked_sub(actual_debt_units).ok_or(Error::ArithmeticError)?;
                    self.env().transfer(liquidator, excess).map_err(|_| Error::TransferFailed)?;
                }
            } else {
                self.tusdt
                    .transfer_from(liquidator, self.env().account_id(), actual_debt_units)
                    .map_err(|_| Error::TokenTransferFromFailed)?;
            }

            // Transfer alpha collateral to liquidator
            self.env()
                .extension()
                .transfer_stake(
                    liquidator,
                    self.pool_hotkey,
                    collateral_netuid,
                    collateral_netuid,
                    alpha_to_seize,
                )
                .map_err(|_| Error::ChainExtensionFailed)?;

            self.env().emit_event(Liquidated {
                user: borrower,
                collateral_netuid,
                debt_market,
                debt_covered: actual_debt_units,
                collateral_alpha: alpha_to_seize,
                liquidator,
            });

            self.set_idle();
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // Permissionless fee claims
        // ─────────────────────────────────────────────────────────────

        /// Claims accumulated alpha staking yield. 25% (performance fee) is unstaked to TAO
        /// and sent to the treasury. 75% is credited to the per-netuid yield index, increasing
        /// all borrowers' effective collateral proportionally.
        #[ink(message)]
        pub fn claim_alpha_yield(&mut self, netuid: u16) -> Result<()> {
            self.ensure_idle()?;
            self.ensure_approved_netuid(netuid).inspect_err(|_| {
                self.set_idle();
            })?;

            // Get actual available stake
            let availability = self
                .env()
                .extension()
                .get_stake_availability(self.env().account_id(), netuid)
                .map_err(|_| {
                    self.set_idle();
                    Error::ChainExtensionFailed
                })?;

            // Compute booked collateral
            let total_principal = self.netuid_total_collateral.get(netuid).unwrap_or_default();
            let yield_index = self.netuid_yield_index.get(netuid).unwrap_or(Ratio::one());
            let booked = yield_index
                .checked_mul_value(total_principal.into())
                .and_then(|v| Balance::try_from(v).ok())
                .unwrap_or(0);

            // Excess = actual available − booked
            if availability.available <= booked {
                self.set_idle();
                return Ok(()); // no excess, no-op
            }
            let excess =
                availability.available.checked_sub(booked).ok_or(Error::ArithmeticError)?;

            // Split: 25% → treasury (via unstake), 75% → borrowers (stays staked, index grows)
            let fee_alpha = self
                .global_params
                .performance_fee
                .checked_mul_value(excess.into())
                .and_then(|v| Balance::try_from(v).ok())
                .unwrap_or(0);
            let credited = excess.saturating_sub(fee_alpha);

            let tao_received = if fee_alpha > 0 {
                let balance_before = self.env().balance();
                self.env().extension().remove_stake(self.pool_hotkey, netuid, fee_alpha).map_err(
                    |_| {
                        self.set_idle();
                        Error::ChainExtensionFailed
                    },
                )?;
                let balance_after = self.env().balance();
                let tao =
                    balance_after.checked_sub(balance_before).ok_or(Error::ArithmeticError)?;
                if tao > 0 {
                    self.env().transfer(self.treasury, tao).map_err(|_| Error::TransferFailed)?;
                }
                tao
            } else {
                0
            };

            // Update yield index: new_index = (booked + credited) / total_principal
            let index_before = yield_index.into_inner();
            if credited > 0 && total_principal > 0 {
                let new_booked = booked.checked_add(credited).ok_or(Error::ArithmeticError)?;
                let new_index = Ratio::from_integer(new_booked.into())
                    .checked_div_int(total_principal.into())
                    .ok_or(Error::ArithmeticError)?;
                self.netuid_yield_index.insert(netuid, &new_index);
                self.env().emit_event(AlphaYieldClaimed {
                    netuid,
                    excess_alpha: excess,
                    performance_fee_alpha: fee_alpha,
                    tao_received,
                    index_before,
                    index_after: new_index.into_inner(),
                });
            } else if fee_alpha > 0 {
                self.env().emit_event(AlphaYieldClaimed {
                    netuid,
                    excess_alpha: excess,
                    performance_fee_alpha: fee_alpha,
                    tao_received,
                    index_before,
                    index_after: index_before,
                });
            }

            self.set_idle();
            Ok(())
        }

        /// Claims accumulated protocol reserve fees for a supply/borrow market.
        /// Sends the fees to the treasury. Permissionless.
        #[ink(message)]
        pub fn claim_reserve(&mut self, market_id: u8) -> Result<()> {
            self.ensure_idle()?;

            self.accrue_interest(market_id).inspect_err(|_| {
                self.set_idle();
            })?;

            let mut state = self.markets.get(market_id).ok_or(Error::MarketNotFound)?;
            let claimable = state.reserve_accrued;
            if claimable == 0 {
                self.set_idle();
                return Ok(());
            }

            // Cap at physical cash available
            let cash = self.market_cash(market_id)?;
            let claim = min(claimable, cash);

            state.reserve_accrued =
                state.reserve_accrued.checked_sub(claim).ok_or(Error::ArithmeticError)?;
            self.markets.insert(market_id, &state);

            match market_id {
                0 => {
                    self.env().transfer(self.treasury, claim).map_err(|_| Error::TransferFailed)?;
                },
                1 => {
                    self.tusdt
                        .transfer(self.treasury, claim)
                        .map_err(|_| Error::TokenContractCallFailed)?;
                },
                _ => {
                    self.set_idle();
                    return Err(Error::MarketNotFound);
                },
            }

            self.env().emit_event(ReserveClaimed { market: market_id, amount: claim });

            self.set_idle();
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // Governance: alpha market management
        // ─────────────────────────────────────────────────────────────

        /// Adds or removes a subnet from the approved alpha markets list.
        #[ink(message)]
        pub fn set_approved_netuid(&mut self, netuid: u16, approved: bool) -> Result<()> {
            self.ensure_maintainer()?;

            if approved {
                if self.netuid_to_market.get(netuid).is_some() {
                    return Ok(()); // already approved
                }
                let market_id = self.next_alpha_market_id;
                self.netuid_to_market.insert(netuid, &market_id);
                self.market_to_netuid.insert(market_id, &netuid);
                self.market_keys.push(&market_id);
                self.next_alpha_market_id =
                    market_id.checked_add(1).ok_or(Error::ArithmeticError)?;
                self.env().emit_event(NetuidApproved {
                    netuid,
                    approved: true,
                    market_id: Some(market_id),
                });
            } else {
                let market_id = match self.netuid_to_market.get(netuid) {
                    Some(id) => id,
                    None => return Ok(()), // already not approved
                };
                // Refuse if any user still holds alpha on this netuid
                let total = self.netuid_total_collateral.get(netuid).unwrap_or_default();
                if total > 0 {
                    return Err(Error::NetuidHasPositions);
                }
                self.netuid_to_market.remove(netuid);
                self.market_to_netuid.remove(market_id);
                self.env().emit_event(NetuidApproved { netuid, approved: false, market_id: None });
            }
            Ok(())
        }

        /// Sets alpha market parameters (schedules timelocked update).
        #[ink(message)]
        pub fn set_alpha_params(
            &mut self,
            netuid: u16,
            config: AlphaMarketParamsConfig,
        ) -> Result<()> {
            self.ensure_maintainer()?;
            self.ensure_approved_netuid(netuid)?;

            let params = Self::alpha_params_from_config(config)?;
            Self::validate_alpha_params(&params)?;

            let execute_after = self
                .env()
                .block_timestamp()
                .checked_add(PARAMS_TIMELOCK_MS)
                .ok_or(Error::ArithmeticError)?;
            self.pending_alpha_params
                .insert(netuid, &PendingAlphaParamsUpdate { params: config, execute_after });

            self.env().emit_event(AlphaParamsUpdateScheduled { netuid, execute_after });
            Ok(())
        }

        /// Executes a scheduled alpha params update (permissionless, time-gated).
        #[ink(message)]
        pub fn execute_alpha_params_update(&mut self, netuid: u16) -> Result<()> {
            let pending =
                self.pending_alpha_params.get(netuid).ok_or(Error::NoPendingAlphaParamsUpdate)?;
            let now = self.env().block_timestamp();
            if now < pending.execute_after {
                return Err(Error::ParamsUpdateTimelockActive);
            }
            let params = Self::alpha_params_from_config(pending.params)?;
            Self::validate_alpha_params(&params)?;
            self.alpha_params.insert(netuid, &params);
            self.pending_alpha_params.remove(netuid);
            self.env().emit_event(AlphaParamsUpdated { netuid });
            Ok(())
        }

        /// Cancels a scheduled alpha params update.
        #[ink(message)]
        pub fn cancel_alpha_params_update(&mut self, netuid: u16) -> Result<()> {
            self.ensure_maintainer()?;
            if self.pending_alpha_params.get(netuid).is_none() {
                return Err(Error::NoPendingAlphaParamsUpdate);
            }
            self.pending_alpha_params.remove(netuid);
            self.env().emit_event(AlphaParamsUpdateCancelled { netuid });
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // Governance: interest rate params (timelocked)
        // ─────────────────────────────────────────────────────────────

        #[ink(message)]
        pub fn set_market_params(
            &mut self,
            market_id: u8,
            config: InterestRateParamsConfig,
        ) -> Result<()> {
            self.ensure_maintainer()?;
            if market_id > 1 {
                return Err(Error::MarketNotFound);
            }
            let params = Self::interest_params_from_config(config)?;
            Self::validate_interest_params(&params)?;

            let execute_after = self
                .env()
                .block_timestamp()
                .checked_add(PARAMS_TIMELOCK_MS)
                .ok_or(Error::ArithmeticError)?;
            self.pending_market_params
                .insert(market_id, &PendingInterestParamsUpdate { params: config, execute_after });
            self.env().emit_event(MarketParamsUpdateScheduled { market: market_id, execute_after });
            Ok(())
        }

        #[ink(message)]
        pub fn execute_market_params_update(&mut self, market_id: u8) -> Result<()> {
            let pending = self
                .pending_market_params
                .get(market_id)
                .ok_or(Error::NoPendingMarketParamsUpdate)?;
            let now = self.env().block_timestamp();
            if now < pending.execute_after {
                return Err(Error::ParamsUpdateTimelockActive);
            }
            let params = Self::interest_params_from_config(pending.params)?;
            Self::validate_interest_params(&params)?;
            self.market_params.insert(market_id, &params);
            self.pending_market_params.remove(market_id);
            self.env().emit_event(MarketParamsUpdated { market: market_id });
            Ok(())
        }

        #[ink(message)]
        pub fn cancel_market_params_update(&mut self, market_id: u8) -> Result<()> {
            self.ensure_maintainer()?;
            if self.pending_market_params.get(market_id).is_none() {
                return Err(Error::NoPendingMarketParamsUpdate);
            }
            self.pending_market_params.remove(market_id);
            self.env().emit_event(MarketParamsUpdateCancelled { market: market_id });
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // Governance: global params (timelocked)
        // ─────────────────────────────────────────────────────────────

        #[ink(message)]
        pub fn set_global_params(&mut self, config: PoolGlobalParamsConfig) -> Result<()> {
            self.ensure_maintainer()?;
            let params = Self::global_params_from_config(config)?;
            Self::validate_global_params(&params)?;

            let execute_after = self
                .env()
                .block_timestamp()
                .checked_add(PARAMS_TIMELOCK_MS)
                .ok_or(Error::ArithmeticError)?;
            self.pending_global_params =
                Some(PendingGlobalParamsUpdate { params: config, execute_after });
            self.env().emit_event(GlobalParamsUpdateScheduled { execute_after });
            Ok(())
        }

        #[ink(message)]
        pub fn execute_global_params_update(&mut self) -> Result<()> {
            let pending = self.pending_global_params.ok_or(Error::NoPendingGlobalParamsUpdate)?;
            let now = self.env().block_timestamp();
            if now < pending.execute_after {
                return Err(Error::ParamsUpdateTimelockActive);
            }
            let params = Self::global_params_from_config(pending.params)?;
            Self::validate_global_params(&params)?;
            self.global_params = params;
            self.pending_global_params = None;
            self.env().emit_event(GlobalParamsUpdated {});
            Ok(())
        }

        #[ink(message)]
        pub fn cancel_global_params_update(&mut self) -> Result<()> {
            self.ensure_maintainer()?;
            if self.pending_global_params.is_none() {
                return Err(Error::NoPendingGlobalParamsUpdate);
            }
            self.pending_global_params = None;
            self.env().emit_event(GlobalParamsUpdateCancelled {});
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // Governance: role and address management
        // ─────────────────────────────────────────────────────────────

        #[ink(message)]
        pub fn update_governance(&mut self, new_governance: AccountId) -> Result<()> {
            self.ensure_governance()?;
            let previous = self.governance;
            self.governance = new_governance;
            self.env().emit_event(PoolGovernanceUpdated { previous, new: new_governance });
            Ok(())
        }

        #[ink(message)]
        pub fn update_maintainer(&mut self, new_maintainer: AccountId) -> Result<()> {
            self.ensure_governance()?;
            self.maintainer = new_maintainer;
            self.env().emit_event(PoolMaintainerUpdated { new_maintainer });
            Ok(())
        }

        #[ink(message)]
        pub fn maintainer(&self) -> AccountId {
            self.maintainer
        }

        #[ink(message)]
        pub fn update_treasury(&mut self, new_treasury: AccountId) -> Result<()> {
            self.ensure_governance()?;
            let previous = self.treasury;
            self.treasury = new_treasury;
            self.env().emit_event(PoolTreasuryUpdated { previous, new: new_treasury });
            Ok(())
        }

        #[ink(message)]
        pub fn update_platform(&mut self, new_platform: AccountId) -> Result<()> {
            self.ensure_maintainer()?;
            let previous = self.platform;
            self.platform = new_platform;
            self.env().emit_event(PoolPlatformUpdated { previous, new: new_platform });
            Ok(())
        }

        #[ink(message)]
        pub fn update_oracle_address(&mut self, new_oracle: AccountId) -> Result<()> {
            self.ensure_maintainer()?;
            let previous = self.oracle.to_account_id();
            self.oracle = TusdtOracleRef::from_account_id(new_oracle);
            self.env().emit_event(PoolOracleAddressUpdated { previous, new: new_oracle });
            Ok(())
        }

        #[ink(message)]
        pub fn update_ltoken_address(
            &mut self,
            market_id: u8,
            new_ltoken: AccountId,
        ) -> Result<()> {
            self.ensure_maintainer()?;
            let old = self.ltoken_by_market.get(market_id).ok_or(Error::MarketNotFound)?;
            self.ltoken_by_market.insert(market_id, &new_ltoken);
            self.env().emit_event(LTokenAddressUpdated {
                market: market_id,
                old_ltoken: old,
                new_ltoken,
            });
            Ok(())
        }

        /// Migrates the pool's staking hotkey, moving all alpha stake to a new hotkey.
        #[ink(message)]
        pub fn update_pool_hotkey(
            &mut self,
            new_hotkey: AccountId,
            netuids: Vec<u16>,
        ) -> Result<()> {
            self.ensure_maintainer()?;
            if netuids.len() > 32 {
                return Err(Error::TooManyNetuids);
            }
            let old_hotkey = self.pool_hotkey;
            for netuid in netuids {
                let total = self.netuid_total_collateral.get(netuid).unwrap_or_default();
                if total > 0 {
                    self.env()
                        .extension()
                        .move_stake(old_hotkey, new_hotkey, netuid, netuid, total)
                        .map_err(|_| Error::ChainExtensionFailed)?;
                }
            }
            self.pool_hotkey = new_hotkey;
            self.env().emit_event(PoolHotkeyChanged { old_hotkey, new_hotkey });
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // Emergency pause
        // ─────────────────────────────────────────────────────────────

        #[ink(message)]
        pub fn pause(&mut self) -> Result<()> {
            self.ensure_governance_or_platform()?;
            self.paused = true;
            self.env().emit_event(PoolPaused {});
            Ok(())
        }

        #[ink(message)]
        pub fn unpause(&mut self) -> Result<()> {
            self.ensure_maintainer()?;
            self.paused = false;
            self.env().emit_event(PoolUnpaused {});
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // Surplus sweeps
        // ─────────────────────────────────────────────────────────────

        #[ink(message)]
        pub fn claim_surplus_tusdt(&mut self, amount: Balance) -> Result<()> {
            self.ensure_maintainer()?;
            self.tusdt
                .transfer(self.treasury, amount)
                .map_err(|_| Error::TokenContractCallFailed)?;
            self.env().emit_event(PoolSurplusTusdtClaimed { recipient: self.treasury, amount });
            Ok(())
        }

        #[ink(message)]
        pub fn transfer_native_to_treasury(&mut self) -> Result<()> {
            self.ensure_maintainer()?;
            let balance = self.env().balance();
            if balance <= 1 {
                return Ok(()); // keep 1 as existential deposit guard
            }
            let amount = balance.checked_sub(1).ok_or(Error::ArithmeticError)?;
            self.env().transfer(self.treasury, amount).map_err(|_| Error::TransferFailed)?;
            self.env().emit_event(PoolNativeTransferredToTreasury { amount });
            Ok(())
        }

        // ─────────────────────────────────────────────────────────────
        // View / read methods
        // ─────────────────────────────────────────────────────────────

        #[ink(message)]
        pub fn get_market_state(&self, market_id: u8) -> Option<MarketState> {
            self.markets.get(market_id)
        }

        #[ink(message)]
        pub fn get_position(&self, market_id: u8, user: AccountId) -> Option<Position> {
            self.positions.get((market_id, user))
        }

        #[ink(message)]
        pub fn get_exchange_rate(&self, market_id: u8) -> Option<Ratio> {
            self.markets.get(market_id).map(|s| s.exchange_rate)
        }

        #[ink(message)]
        pub fn get_borrow_index(&self, market_id: u8) -> Option<Ratio> {
            self.markets.get(market_id).map(|s| s.borrow_index)
        }

        #[ink(message)]
        pub fn get_utilization(&self, market_id: u8) -> Option<Ratio> {
            let state = self.markets.get(market_id)?;
            let cash = self.market_cash(market_id).ok()?;
            let total = state.total_debt.checked_add(cash)?;
            if total == 0 {
                return Some(Ratio::from_inner(0));
            }
            Ratio::from_integer(state.total_debt.into()).checked_div_int(total.into())
        }

        #[ink(message)]
        pub fn get_borrow_rate(&self, market_id: u8) -> Option<Ratio> {
            let params = self.market_params.get(market_id)?;
            let utilization = self.get_utilization(market_id)?;
            Self::compute_borrow_rate(&params, utilization).ok()
        }

        #[ink(message)]
        pub fn get_supply_rate(&self, market_id: u8) -> Option<Ratio> {
            let params = self.market_params.get(market_id)?;
            let utilization = self.get_utilization(market_id)?;
            let borrow_rate = Self::compute_borrow_rate(&params, utilization).ok()?;
            let one = Ratio::one();
            borrow_rate.checked_mul(utilization).and_then(|r| {
                let one_minus_rf = Ratio::from_inner(
                    one.into_inner().checked_sub(params.reserve_factor.into_inner())?,
                );
                r.checked_mul(one_minus_rf)
            })
        }

        #[ink(message)]
        pub fn get_underlying_balance(&self, market_id: u8, user: AccountId) -> Option<Balance> {
            let pos = self.positions.get((market_id, user))?;
            let state = self.markets.get(market_id)?;
            let ltoken_addr = self.ltoken_by_market.get(market_id)?;
            let ltoken = TusdtErc20Ref::from_account_id(ltoken_addr);
            let ltoken_supply = ltoken.total_supply();
            if ltoken_supply == 0 || state.total_supplied == 0 {
                return Some(0);
            }
            Ratio::from_integer(pos.ltoken_balance.into())
                .checked_mul_value(state.total_supplied.into())
                .and_then(|v| v.checked_div(ltoken_supply as u128))
                .and_then(|v| u64::try_from(v).ok())
        }

        #[ink(message)]
        pub fn get_user_debt(&self, market_id: u8, user: AccountId) -> Option<Balance> {
            let pos = self.positions.get((market_id, user)).unwrap_or(Position {
                ltoken_balance: 0,
                scaled_debt: 0,
                alpha_principal: 0,
            });
            let state = self.markets.get(market_id)?;
            if pos.scaled_debt == 0 {
                return Some(0);
            }
            state
                .borrow_index
                .checked_mul_value(pos.scaled_debt.into())
                .and_then(|v| Balance::try_from(v).ok())
        }

        #[ink(message)]
        pub fn get_alpha_markets(&self) -> Vec<(u16, AlphaMarketParams)> {
            let mut result = Vec::new();
            let count = self.market_keys.len();
            for i in 0..count {
                let market_id = match self.market_keys.get(i) {
                    Some(id) => id,
                    None => continue,
                };
                if market_id < 2 {
                    continue;
                }
                let netuid = match self.market_to_netuid.get(market_id) {
                    Some(n) => n,
                    None => continue,
                };
                let params = self.alpha_params.get(netuid).unwrap_or(default_alpha_params());
                result.push((netuid, params));
            }
            result
        }

        #[ink(message)]
        pub fn get_user_alpha_position(&self, user: AccountId, netuid: u16) -> Option<Balance> {
            let market_id = self.netuid_to_market.get(netuid)?;
            let pos = self.positions.get((market_id, user))?;
            Some(pos.alpha_principal)
        }

        #[ink(message)]
        pub fn get_alpha_yield_index(&self, netuid: u16) -> Option<Ratio> {
            Some(self.netuid_yield_index.get(netuid).unwrap_or(Ratio::one()))
        }

        #[ink(message)]
        pub fn get_netuid_total_collateral(&self, netuid: u16) -> Option<Balance> {
            self.netuid_total_collateral.get(netuid)
        }

        #[ink(message)]
        pub fn is_approved_netuid(&self, netuid: u16) -> bool {
            self.netuid_to_market.get(netuid).is_some()
        }

        #[ink(message)]
        pub fn get_active_netuids_count(&self) -> u32 {
            let count = self.market_keys.len();
            let mut alpha_count: u32 = 0;
            for i in 0..count {
                if let Some(id) = self.market_keys.get(i) {
                    if id >= 2 {
                        alpha_count = alpha_count.saturating_add(1);
                    }
                }
            }
            alpha_count
        }

        #[ink(message)]
        pub fn get_positions(&self, user: AccountId, page: u32) -> Vec<(u8, Position)> {
            let mut result = Vec::new();
            let mut seen = Vec::new();
            let total = self.position_keys.len();
            let start = page.saturating_mul(PAGE_SIZE);
            let end = min(start.saturating_add(PAGE_SIZE), total);
            for i in start..end {
                let key = match self.position_keys.get(i) {
                    Some(k) => k,
                    None => continue,
                };
                if key.1 != user {
                    continue;
                }
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                if let Some(pos) = self.positions.get(key) {
                    result.push((key.0, pos));
                }
            }
            result
        }

        #[ink(message)]
        pub fn get_all_positions(&self, page: u32) -> Vec<((u8, AccountId), Position)> {
            let mut result = Vec::new();
            let mut seen = Vec::new();
            let total = self.position_keys.len();
            let start = page.saturating_mul(PAGE_SIZE);
            let end = min(start.saturating_add(PAGE_SIZE), total);
            for i in start..end {
                let key = match self.position_keys.get(i) {
                    Some(k) => k,
                    None => continue,
                };
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                if let Some(pos) = self.positions.get(key) {
                    result.push((key, pos));
                }
            }
            result
        }

        #[ink(message)]
        pub fn get_pending_alpha_params_update(
            &self,
            netuid: u16,
        ) -> Option<PendingAlphaParamsUpdate> {
            self.pending_alpha_params.get(netuid)
        }

        #[ink(message)]
        pub fn get_pending_market_params_update(
            &self,
            market_id: u8,
        ) -> Option<PendingInterestParamsUpdate> {
            self.pending_market_params.get(market_id)
        }

        #[ink(message)]
        pub fn get_pending_global_params_update(&self) -> Option<PendingGlobalParamsUpdate> {
            self.pending_global_params
        }

        // ─────────────────────────────────────────────────────────────
        // Role / address getters
        // ─────────────────────────────────────────────────────────────

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
        pub fn get_oracle_address(&self) -> AccountId {
            self.oracle.to_account_id()
        }

        #[ink(message)]
        pub fn get_tusdt_address(&self) -> AccountId {
            self.tusdt.to_account_id()
        }

        #[ink(message)]
        pub fn get_ltoken_address(&self, market: u8) -> Option<AccountId> {
            self.ltoken_by_market.get(market)
        }

        #[ink(message)]
        pub fn get_pool_hotkey(&self) -> AccountId {
            self.pool_hotkey
        }

        #[ink(message)]
        pub fn get_global_params(&self) -> PoolGlobalParamsConfig {
            self.global_params.to_config()
        }

        #[ink(message)]
        pub fn get_market_params(&self, market_id: u8) -> Option<InterestRateParamsConfig> {
            self.market_params.get(market_id).map(|p| p.to_config())
        }

        #[ink(message)]
        pub fn get_alpha_params(&self, netuid: u16) -> Option<AlphaMarketParamsConfig> {
            self.alpha_params.get(netuid).map(|p| p.to_config())
        }
    }

    #[cfg(test)]
    impl TusdtLendingPool {
        /// Test-only constructor: builds a pool with placeholder contract addresses
        /// so unit tests can run without deployed child/oracle instances.
        pub(crate) fn new_for_test(governance: AccountId) -> Self {
            use ink::env::call::FromAccountId;
            let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
            let now = 0u64;

            let mut markets = Mapping::default();
            markets.insert(0, &MarketState::new(now));
            markets.insert(1, &MarketState::new(now));

            let mut ltoken_by_market = Mapping::default();
            ltoken_by_market.insert(0, &accounts.charlie); // fake lTAO
            ltoken_by_market.insert(1, &accounts.eve); // fake lTUSDT

            let mut market_keys = StorageVec::new();
            market_keys.push(&0);
            market_keys.push(&1);

            let mut market_params = Mapping::default();
            market_params.insert(0, &default_tao_interest_params());
            market_params.insert(1, &default_tusdt_interest_params());

            Self {
                governance,
                maintainer: governance,
                treasury: governance,
                platform: governance,
                paused: false,
                busy: false,
                oracle: TusdtOracleRef::from_account_id(accounts.frank),
                tusdt: TusdtErc20Ref::from_account_id(accounts.django),
                pool_hotkey: accounts.bob,
                markets,
                market_keys,
                ltoken_by_market,
                netuid_to_market: Mapping::default(),
                market_to_netuid: Mapping::default(),
                next_alpha_market_id: 2,
                market_params,
                alpha_params: Mapping::default(),
                pending_market_params: Mapping::default(),
                pending_alpha_params: Mapping::default(),
                global_params: default_global_params(),
                pending_global_params: None,
                netuid_total_collateral: Mapping::default(),
                netuid_yield_index: Mapping::default(),
                positions: Mapping::default(),
                position_keys: StorageVec::new(),
            }
        }

        /// Test-only: directly set a position in the mapping.
        pub(crate) fn debug_set_position(&mut self, market_id: u8, user: AccountId, pos: Position) {
            self.positions.insert((market_id, user), &pos);
        }

        /// Test-only: push a key into position_keys (simulates legacy
        /// append-only behaviour for testing dedup and self-healing).
        pub(crate) fn debug_push_position_key(&mut self, market_id: u8, user: AccountId) {
            self.position_keys.push(&(market_id, user));
        }
    }
}

#[cfg(test)]
mod tests {
    include!("tests.rs");
}
