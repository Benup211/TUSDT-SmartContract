use super::*;

const DEFAULT_COLLATERAL_RATIO_BASIS_POINTS: u32 = 15_000;
const DEFAULT_LIQUIDATION_RATIO_BASIS_POINTS: u32 = 12_000;
const DEFAULT_LIQUIDATION_FEE_BASIS_POINTS: u32 = 1_100;
const DEFAULT_TRANSACTION_FEE_BASIS_POINTS: u32 = 30;
const DEFAULT_AUCTION_DURATION_MS: u64 = 3_600_000;
const DEFAULT_MAX_ORACLE_AGE_MS: u64 = 1_800_000;
const DEFAULT_VAULT_CREATION_FEE: Balance = 5_000_000;
const MIN_AUCTION_DURATION_MS: u64 = 60_000;
const MAX_AUCTION_DURATION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

impl TusdtVaultAlpha {
    /// Returns the default per-netuid vault parameters.
    ///
    /// Defaults: collateral ratio 150%, liquidation ratio 120%,
    /// liquidation fee 11%.
    pub(crate) fn default_contract_params() -> VaultContractParams {
        let params = VaultContractParams {
            collateral_ratio: Ratio::from_basis_points(DEFAULT_COLLATERAL_RATIO_BASIS_POINTS),
            liquidation_ratio: Ratio::from_basis_points(DEFAULT_LIQUIDATION_RATIO_BASIS_POINTS),
            liquidation_fee: Ratio::from_basis_points(DEFAULT_LIQUIDATION_FEE_BASIS_POINTS),
        };
        Self::validate_contract_params(&params)
            .expect("default vault contract params should be valid");
        params
    }

    /// Converts an external bps-based `VaultContractParamsConfig` into internal
    /// `VaultContractParams` values, validating all ratio bounds.
    pub(crate) fn contract_params_from_config(
        params: VaultContractParamsConfig,
    ) -> Result<VaultContractParams> {
        let config = VaultContractParams {
            collateral_ratio: Ratio::from_basis_points(params.collateral_ratio),
            liquidation_ratio: Ratio::from_basis_points(params.liquidation_ratio),
            liquidation_fee: Ratio::from_basis_points(params.liquidation_fee),
        };
        Self::validate_contract_params(&config)?;
        Ok(config)
    }

    /// Converts internal `VaultContractParams` back into a bps-based
    /// `VaultContractParamsConfig` for external consumption.
    pub(crate) fn contract_params_to_config(
        params: VaultContractParams,
    ) -> VaultContractParamsConfig {
        VaultContractParamsConfig {
            collateral_ratio: params
                .collateral_ratio
                .to_basis_points()
                .expect("stored collateral ratio should fit in u32 basis points"),
            liquidation_ratio: params
                .liquidation_ratio
                .to_basis_points()
                .expect("stored liquidation ratio should fit in u32 basis points"),
            liquidation_fee: params
                .liquidation_fee
                .to_basis_points()
                .expect("stored liquidation fee should fit in u32 basis points"),
        }
    }

    /// Validates per-netuid contract parameters.
    ///
    /// - `collateral_ratio` must be in [100%, 1_000_000%] and strictly greater than `liquidation_ratio`
    /// - `liquidation_ratio` must be in [100%, 1_000_000%]
    /// - `liquidation_fee` must be <= 100%
    pub(crate) fn validate_contract_params(params: &VaultContractParams) -> Result<()> {
        let one = Ratio::one();
        // Upper bound prevents panic in to_basis_points() which fails above ~429,496%.
        // 1_000_000% (10,000,000 bps) is a safe ceiling well below the conversion limit.
        let max_ratio = Ratio::from_basis_points(10_000_000);
        if params.collateral_ratio < one || params.collateral_ratio > max_ratio {
            return Err(Error::InvalidRatio);
        }
        if params.liquidation_ratio < one || params.liquidation_ratio > max_ratio {
            return Err(Error::InvalidRatio);
        }
        if params.collateral_ratio <= params.liquidation_ratio {
            return Err(Error::InvalidRatio);
        }
        if params.liquidation_fee > one {
            return Err(Error::InvalidRatio);
        }
        Ok(())
    }

    /// Returns the default global (contract-wide) parameters.
    ///
    /// Defaults: transaction fee 0.3% (30 bps), auction duration 1 hour,
    /// max oracle age 30 minutes, vault creation fee 5,000,000 rao (0.005 TAO).
    pub(crate) fn default_global_params() -> VaultGlobalParams {
        let params = VaultGlobalParams {
            transaction_fee: Ratio::from_basis_points(DEFAULT_TRANSACTION_FEE_BASIS_POINTS),
            auction_duration_ms: DEFAULT_AUCTION_DURATION_MS,
            max_oracle_age_ms: DEFAULT_MAX_ORACLE_AGE_MS,
            vault_creation_fee: DEFAULT_VAULT_CREATION_FEE,
        };
        Self::validate_global_params(&params)
            .expect("default vault global params should be valid");
        params
    }

    /// Converts an external bps-based `VaultGlobalParamsConfig` into internal
    /// `VaultGlobalParams` values, validating all fields.
    pub(crate) fn global_params_from_config(
        config: VaultGlobalParamsConfig,
    ) -> Result<VaultGlobalParams> {
        let params = VaultGlobalParams {
            transaction_fee: Ratio::from_basis_points(config.transaction_fee),
            auction_duration_ms: config.auction_duration_ms,
            max_oracle_age_ms: config.max_oracle_age_ms,
            vault_creation_fee: config.vault_creation_fee,
        };
        Self::validate_global_params(&params)?;
        Ok(params)
    }

    /// Converts internal `VaultGlobalParams` back into a bps-based
    /// `VaultGlobalParamsConfig` for external consumption.
    pub(crate) fn global_params_to_config(params: VaultGlobalParams) -> VaultGlobalParamsConfig {
        VaultGlobalParamsConfig {
            transaction_fee: params
                .transaction_fee
                .to_basis_points()
                .expect("stored transaction fee should fit in u32 basis points"),
            auction_duration_ms: params.auction_duration_ms,
            max_oracle_age_ms: params.max_oracle_age_ms,
            vault_creation_fee: params.vault_creation_fee,
        }
    }

    /// Validates global (contract-wide) parameters.
    ///
    /// - `transaction_fee` must not exceed 100%
    /// - `auction_duration_ms` must be within [60 seconds, 7 days]
    /// - `max_oracle_age_ms` must be non-zero
    /// - `vault_creation_fee` may be zero (disables the fee for testing)
    pub(crate) fn validate_global_params(params: &VaultGlobalParams) -> Result<()> {
        if params.transaction_fee > Ratio::one() {
            return Err(Error::InvalidRatio);
        }
        if params.auction_duration_ms < MIN_AUCTION_DURATION_MS {
            return Err(Error::InvalidAuctionDuration);
        }
        if params.auction_duration_ms > MAX_AUCTION_DURATION_MS {
            return Err(Error::InvalidAuctionDuration);
        }
        if params.max_oracle_age_ms == 0 {
            return Err(Error::InvalidOracleMaxAge);
        }
        Ok(())
    }
}
