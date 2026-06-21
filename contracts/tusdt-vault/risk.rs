use super::*;

impl TusdtVault {
    /// Computes the borrow-denominated value of a collateral balance at the given oracle price.
    ///
    /// Equivalent to `collateral_balance * price`, returning the result as a `Balance`
    /// in tUSDT units (the borrow denomination).
    pub(crate) fn collateral_value(price: Ratio, collateral_balance: Balance) -> Result<Balance> {
        let collateral_value_in_borrow = price
            .checked_mul_value(u128::from(collateral_balance))
            .ok_or(Error::ArithmeticError)?;
        Balance::try_from(collateral_value_in_borrow).map_err(|_| Error::ArithmeticError)
    }

    /// Returns the maximum amount a vault can borrow against its collateral at the given price.
    ///
    /// Computed as `collateral_value / collateral_ratio`, where the collateral ratio
    /// (e.g. 150%) enforces an over-collateralization minimum.
    pub(crate) fn max_borrow_allowed(
        &self,
        price: Ratio,
        collateral_balance: Balance,
    ) -> Result<Balance> {
        let collateral_value_in_borrow = Self::collateral_value(price, collateral_balance)?;
        let max = self
            .params
            .collateral_ratio
            .checked_div_value(u128::from(collateral_value_in_borrow))
            .ok_or(Error::ArithmeticError)?;
        Balance::try_from(max).map_err(|_| Error::ArithmeticError)
    }

    /// Returns the debt threshold above which a vault becomes eligible for liquidation.
    ///
    /// Computed as `collateral_value / liquidation_ratio`. When debt exceeds this limit,
    /// [`is_liquidatable`](Self::is_liquidatable) returns `true`.
    pub(crate) fn liquidation_limit(
        &self,
        price: Ratio,
        collateral_balance: Balance,
    ) -> Result<Balance> {
        let collateral_value_in_borrow = Self::collateral_value(price, collateral_balance)?;
        let limit = self
            .params
            .liquidation_ratio
            .checked_div_value(u128::from(collateral_value_in_borrow))
            .ok_or(Error::ArithmeticError)?;
        Balance::try_from(limit).map_err(|_| Error::ArithmeticError)
    }

    /// Checks whether a vault's debt balance exceeds the liquidation limit at the given price.
    ///
    /// Returns `true` when `debt_balance > liquidation_limit(price, collateral_balance)`,
    /// meaning a liquidation auction can be triggered.
    pub(crate) fn is_liquidatable(&self, price: Ratio, vault: &Vault) -> Result<bool> {
        let limit = self.liquidation_limit(price, vault.collateral_balance)?;
        Ok(vault.debt_balance > limit)
    }

    /// Computes the minimum winning bid for a liquidation auction.
    ///
    /// The minimum bid equals `debt_balance + liquidation_fee`, where the liquidation fee
    /// is a percentage (e.g. 1%) of the outstanding debt. This ensures the protocol
    /// recovers the full debt plus a penalty to cover the cost of liquidation.
    pub(crate) fn liquidation_min_bid(&self, debt_balance: Balance) -> Result<Balance> {
        let liquidation_fee = self
            .params
            .liquidation_fee
            .checked_mul_value(u128::from(debt_balance))
            .ok_or(Error::ArithmeticError)?;
        let min_bid = u128::from(debt_balance)
            .checked_add(liquidation_fee)
            .ok_or(Error::ArithmeticError)?;

        Balance::try_from(min_bid).map_err(|_| Error::ArithmeticError)
    }

    /// Validates a collateral addition (create or top-up) against min, per-vault cap, and global cap.
    /// Returns the projected (vault_balance, total_balance) the caller must commit on success.
    pub(crate) fn ensure_collateral_bounds(
        &self,
        vault_current: Balance,
        addition: Balance,
    ) -> Result<(Balance, Balance)> {
        if addition == 0 {
            return Err(Error::InsufficientCollateral);
        }

        let projected_vault = vault_current
            .checked_add(addition)
            .ok_or(Error::ArithmeticError)?;
        if projected_vault < self.params.min_vault_collateral {
            return Err(Error::InsufficientCollateral);
        }
        if projected_vault > self.params.max_vault_collateral {
            return Err(Error::CollateralCapExceeded);
        }

        let projected_total = self
            .total_collateral_balance
            .checked_add(addition)
            .ok_or(Error::ArithmeticError)?;
        if projected_total > self.params.max_total_collateral {
            return Err(Error::CollateralCapExceeded);
        }

        Ok((projected_vault, projected_total))
    }
}
