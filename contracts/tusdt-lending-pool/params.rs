use super::*;

    /// Timelock delay (ms) that pending parameter updates must wait before
    /// they can be executed (24 hours).
    pub(crate) const PARAMS_TIMELOCK_MS: u64 = 24 * 60 * 60 * 1_000;

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
        /// Converts the internal Ratio-based interest-rate params to the external
        /// basis-points (BPS) config representation.
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
        /// Converts the internal Ratio-based alpha market params to the external
        /// basis-points (BPS) config representation.
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
        /// Converts the internal Ratio-based global params to the external config
        /// representation (ratios as basis points).
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

    /// Default interest-rate parameters for the TAO supply/borrow market.
    /// base=0%, slope1=4%, slope2=96%, optimal=80%, reserve_factor=20%
    pub(crate) fn default_tao_interest_params() -> InterestRateParams {
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
    pub(crate) fn default_tusdt_interest_params() -> InterestRateParams {
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
    pub(crate) fn default_alpha_params() -> AlphaMarketParams {
        AlphaMarketParams {
            collateral_factor: Ratio::from_basis_points(5000),
            liquidation_threshold: Ratio::from_basis_points(6000),
            liquidation_bonus: Ratio::from_basis_points(500),
            supply_cap: 0, // unlimited
        }
    }

    /// Default global parameters.
    pub(crate) fn default_global_params() -> PoolGlobalParams {
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

impl TusdtLendingPool {
        // ─────────────────────────────────────────────────────────────
        // Param validation
        // ─────────────────────────────────────────────────────────────

        /// Validates a basis-points interest-rate config (optimal utilization in
        /// (0, 10_000], slopes summing <= 10_000, reserve factor < 10_000) and
        /// converts it to internal Ratio-based params. Errors:
        /// `Error::InvalidParam`, `Error::ArithmeticError`.
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

        /// Validates internal Ratio-based interest params: max rate <= 100%,
        /// optimal utilization in (0, 100%], reserve factor < 100%. Errors:
        /// `Error::InvalidRatio`, `Error::ArithmeticError`.
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

        /// Validates a basis-points alpha market config (collateral factor <
        /// liquidation threshold <= 10_000, bonus <= 2_500) and converts it to
        /// internal Ratio-based params. Errors: `Error::InvalidParam`.
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

        /// Validates internal Ratio-based alpha params: collateral factor > 0 and
        /// < liquidation threshold <= 100%, bonus <= 25%. Errors:
        /// `Error::InvalidRatio`.
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

        /// Validates a global params config (oracle age > 0, close factor in
        /// (0, 5_000], performance fee <= 5_000) and converts it to internal
        /// Ratio-based params. Errors: `Error::InvalidParam`.
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

        /// Validates internal Ratio-based global params with the same bounds as
        /// `global_params_from_config`. Errors: `Error::InvalidRatio`.
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
}
