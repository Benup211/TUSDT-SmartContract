use super::*;

const DEFAULT_COLLATERAL_RATIO_BASIS_POINTS: u32 = 15_000;
const DEFAULT_LIQUIDATION_RATIO_BASIS_POINTS: u32 = 12_000;
const DEFAULT_INTEREST_RATE_BASIS_POINTS: u32 = 1_000;
const DEFAULT_LIQUIDATION_FEE_BASIS_POINTS: u32 = 1_100;
const DEFAULT_BORROW_CAP: Balance = 5_000_000_000_000;
const DEFAULT_MIN_VAULT_COLLATERAL: Balance = 5_000_000;
const DEFAULT_MAX_VAULT_COLLATERAL: Balance = 1_000_000_000;
const DEFAULT_MAX_TOTAL_COLLATERAL: Balance = 21_000_000_000;
const DEFAULT_TRANSACTION_FEE_BASIS_POINTS: u32 = 30;
const DEFAULT_AUCTION_DURATION_MS: u64 = 3_600_000;
const DEFAULT_MAX_ORACLE_AGE_MS: u64 = 1_800_000;
const MAX_AUCTION_DURATION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

impl TusdtVaultAlpha {
    pub(crate) fn default_contract_params() -> VaultContractParams {
        let params = VaultContractParams {
            collateral_ratio: Ratio::from_basis_points(DEFAULT_COLLATERAL_RATIO_BASIS_POINTS),
            liquidation_ratio: Ratio::from_basis_points(DEFAULT_LIQUIDATION_RATIO_BASIS_POINTS),
            interest_rate: Ratio::from_basis_points(DEFAULT_INTEREST_RATE_BASIS_POINTS),
            liquidation_fee: Ratio::from_basis_points(DEFAULT_LIQUIDATION_FEE_BASIS_POINTS),
            borrow_cap: DEFAULT_BORROW_CAP,
            min_vault_collateral: DEFAULT_MIN_VAULT_COLLATERAL,
            max_vault_collateral: DEFAULT_MAX_VAULT_COLLATERAL,
            max_total_collateral: DEFAULT_MAX_TOTAL_COLLATERAL,
            transaction_fee: Ratio::from_basis_points(DEFAULT_TRANSACTION_FEE_BASIS_POINTS),
            auction_duration_ms: DEFAULT_AUCTION_DURATION_MS,
            max_oracle_age_ms: DEFAULT_MAX_ORACLE_AGE_MS,
        };
        Self::validate_contract_params(&params)
            .expect("default vault contract params should be valid");
        params
    }

    pub(crate) fn contract_params_from_config(
        params: VaultContractParamsConfig,
    ) -> Result<VaultContractParams> {
        let config = VaultContractParams {
            collateral_ratio: Ratio::from_basis_points(params.collateral_ratio),
            liquidation_ratio: Ratio::from_basis_points(params.liquidation_ratio),
            interest_rate: Ratio::from_basis_points(params.interest_rate),
            liquidation_fee: Ratio::from_basis_points(params.liquidation_fee),
            borrow_cap: params.borrow_cap,
            min_vault_collateral: params.min_vault_collateral,
            max_vault_collateral: params.max_vault_collateral,
            max_total_collateral: params.max_total_collateral,
            transaction_fee: Ratio::from_basis_points(params.transaction_fee),
            auction_duration_ms: params.auction_duration_ms,
            max_oracle_age_ms: params.max_oracle_age_ms,
        };
        Self::validate_contract_params(&config)?;
        Ok(config)
    }

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
            interest_rate: params
                .interest_rate
                .to_basis_points()
                .expect("stored interest rate should fit in u32 basis points"),
            liquidation_fee: params
                .liquidation_fee
                .to_basis_points()
                .expect("stored liquidation fee should fit in u32 basis points"),
            borrow_cap: params.borrow_cap,
            min_vault_collateral: params.min_vault_collateral,
            max_vault_collateral: params.max_vault_collateral,
            max_total_collateral: params.max_total_collateral,
            transaction_fee: params
                .transaction_fee
                .to_basis_points()
                .expect("stored transaction fee should fit in u32 basis points"),
            auction_duration_ms: params.auction_duration_ms,
            max_oracle_age_ms: params.max_oracle_age_ms,
        }
    }

    pub(crate) fn validate_contract_params(params: &VaultContractParams) -> Result<()> {
        let one = Ratio::one();
        if params.collateral_ratio < one {
            return Err(Error::InvalidRatio);
        }
        if params.liquidation_ratio < one {
            return Err(Error::InvalidRatio);
        }
        if params.collateral_ratio <= params.liquidation_ratio {
            return Err(Error::InvalidRatio);
        }
        if params.interest_rate > one {
            return Err(Error::InvalidRatio);
        }
        if params.liquidation_fee > one {
            return Err(Error::InvalidRatio);
        }
        if params.transaction_fee > one {
            return Err(Error::InvalidRatio);
        }
        if params.auction_duration_ms < 60_000 {
            return Err(Error::InvalidAuctionDuration);
        }
        if params.auction_duration_ms > MAX_AUCTION_DURATION_MS {
            return Err(Error::InvalidAuctionDuration);
        }
        if params.max_oracle_age_ms == 0 {
            return Err(Error::InvalidOracleMaxAge);
        }
        if params.min_vault_collateral == 0 {
            return Err(Error::InvalidRatio);
        }
        if params.max_vault_collateral < params.min_vault_collateral {
            return Err(Error::InvalidRatio);
        }
        if params.max_total_collateral < params.max_vault_collateral {
            return Err(Error::InvalidRatio);
        }
        Ok(())
    }
}
