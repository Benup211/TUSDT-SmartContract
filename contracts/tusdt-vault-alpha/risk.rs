use super::*;

impl TusdtVaultAlpha {
    /// Computes the borrow-denominated value of a collateral balance at the given oracle price.
    pub(crate) fn collateral_value(price: Ratio, collateral_balance: Balance) -> Result<Balance> {
        let collateral_value_in_borrow = price
            .checked_mul_value(u128::from(collateral_balance))
            .ok_or(Error::ArithmeticError)?;
        Balance::try_from(collateral_value_in_borrow).map_err(|_| Error::ArithmeticError)
    }

    /// Returns the maximum amount a vault can borrow against its collateral at the given price,
    /// using the risk parameters configured for the specified netuid.
    pub(crate) fn max_borrow_allowed(
        &self,
        netuid: u16,
        price: Ratio,
        collateral_balance: Balance,
    ) -> Result<Balance> {
        let params = self.get_params(netuid);
        let collateral_value_in_borrow = Self::collateral_value(price, collateral_balance)?;
        let max = params
            .collateral_ratio
            .checked_div_value(u128::from(collateral_value_in_borrow))
            .ok_or(Error::ArithmeticError)?;
        Balance::try_from(max).map_err(|_| Error::ArithmeticError)
    }

    /// Returns the debt threshold above which a vault becomes eligible for liquidation.
    pub(crate) fn liquidation_limit(
        &self,
        netuid: u16,
        price: Ratio,
        collateral_balance: Balance,
    ) -> Result<Balance> {
        let params = self.get_params(netuid);
        let collateral_value_in_borrow = Self::collateral_value(price, collateral_balance)?;
        let limit = params
            .liquidation_ratio
            .checked_div_value(u128::from(collateral_value_in_borrow))
            .ok_or(Error::ArithmeticError)?;
        Balance::try_from(limit).map_err(|_| Error::ArithmeticError)
    }

    /// Checks whether a vault's debt balance exceeds the liquidation limit.
    pub(crate) fn is_liquidatable(&self, price: Ratio, vault: &Vault) -> Result<bool> {
        let limit = self.liquidation_limit(vault.netuid, price, vault.collateral_balance)?;
        Ok(vault.debt_balance > limit)
    }

    /// Computes the minimum winning bid for a liquidation auction, using the fee rate
    /// configured for the specified netuid.
    pub(crate) fn liquidation_min_bid(
        &self,
        netuid: u16,
        debt_balance: Balance,
    ) -> Result<Balance> {
        let params = self.get_params(netuid);
        let liquidation_fee = params
            .liquidation_fee
            .checked_mul_value(u128::from(debt_balance))
            .ok_or(Error::ArithmeticError)?;
        let min_bid = u128::from(debt_balance)
            .checked_add(liquidation_fee)
            .ok_or(Error::ArithmeticError)?;

        Balance::try_from(min_bid).map_err(|_| Error::ArithmeticError)
    }

    /// Validates a collateral addition against min, per-vault cap, and global cap using
    /// the bounds configured for the specified netuid.
    pub(crate) fn ensure_collateral_bounds(
        &self,
        netuid: u16,
        vault_current: Balance,
        addition: Balance,
    ) -> Result<(Balance, Balance)> {
        if addition == 0 {
            return Err(Error::InsufficientCollateral);
        }

        let params = self.get_params(netuid);

        let projected_vault = vault_current
            .checked_add(addition)
            .ok_or(Error::ArithmeticError)?;
        if projected_vault < params.min_vault_collateral {
            return Err(Error::InsufficientCollateral);
        }
        if projected_vault > params.max_vault_collateral {
            return Err(Error::CollateralCapExceeded);
        }

        let netuid_total = self
            .netuid_total_collateral
            .get(netuid)
            .unwrap_or_default();
        let projected_total = netuid_total
            .checked_add(addition)
            .ok_or(Error::ArithmeticError)?;
        if projected_total > params.max_total_collateral {
            return Err(Error::CollateralCapExceeded);
        }

        Ok((projected_vault, projected_total))
    }
}
