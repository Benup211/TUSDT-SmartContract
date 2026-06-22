use super::*;
use ink::codegen::Env as _;
use tusdt_primitives::{HOURS_PER_YEAR, MILLISECONDS_PER_HOUR};

impl TusdtVault {
    /// Accrues hourly-compounded interest on a vault's debt balance since the last accrual.
    ///
    /// Computes the number of whole hours elapsed since `vault.last_interest_accrued_at`,
    /// then applies discrete compound growth at the configured annual rate:
    /// `(1 + rate / 8760)^elapsed_hours`. Advances `last_interest_accrued_at` past the
    /// fully-accrued hours so that partial-hour remainders are picked up on the next call.
    /// Returns the interest delta (the increase in debt balance).
    ///
    /// This is a no-op when debt is zero, the interest rate is zero, or the block
    /// timestamp has not advanced past the last accrual point.
    pub(crate) fn accrue_interest_for_vault(&self, vault: &mut Vault) -> Result<()> {
        let now = self.env().block_timestamp();
        if now <= vault.last_interest_accrued_at {
            return Ok(());
        }
        if vault.debt_balance == 0 || self.params.interest_rate.is_zero() {
            vault.last_interest_accrued_at = now;
            return Ok(());
        }

        // We checked that now > vault.last_interest_accrued_at.
        #[allow(clippy::arithmetic_side_effects)]
        let elapsed = (now - vault.last_interest_accrued_at) as u128;
        let borrowed_hours = elapsed
            .checked_div(MILLISECONDS_PER_HOUR as u128)
            .ok_or(Error::ArithmeticError)?
            .saturating_add(1); // Charge interest at the beginning of each hour.
        if borrowed_hours == 0 {
            return Ok(());
        }

        let hourly_rate = self
            .params
            .interest_rate
            .checked_div_int(HOURS_PER_YEAR)
            .ok_or(Error::ArithmeticError)?;

        let hourly_growth_factor = Ratio::from_inner(
            Ratio::one()
                .into_inner()
                .checked_add(hourly_rate.into_inner())
                .ok_or(Error::ArithmeticError)?,
        );
        let compounded_growth_factor = hourly_growth_factor
            .checked_pow(borrowed_hours)
            .ok_or(Error::ArithmeticError)?;

        let previous_debt_balance = vault.debt_balance;
        let next_debt_balance = compounded_growth_factor
            .checked_mul_value(u128::from(previous_debt_balance))
            .ok_or(Error::ArithmeticError)?;
        let next_debt_balance = core::cmp::min(next_debt_balance, u128::from(Balance::MAX));
        let next_debt_balance =
            Balance::try_from(next_debt_balance).map_err(|_| Error::ArithmeticError)?;
        let interest_accrued = next_debt_balance
            .checked_sub(previous_debt_balance)
            .ok_or(Error::ArithmeticError)?;
        vault.debt_balance = next_debt_balance;
        vault.total_interest_accrued = vault
            .total_interest_accrued
            .checked_add(interest_accrued)
            .ok_or(Error::ArithmeticError)?;

        let accrued_milliseconds = borrowed_hours
            .checked_mul(MILLISECONDS_PER_HOUR as u128)
            .ok_or(Error::ArithmeticError)?
            .checked_add(vault.last_interest_accrued_at as u128)
            .ok_or(Error::ArithmeticError)?;
        if accrued_milliseconds > u64::MAX as u128 {
            return Err(Error::ArithmeticError);
        }
        // We already check max value
        #[allow(clippy::cast_possible_truncation)]
        let accrued_milliseconds = accrued_milliseconds as u64;
        vault.last_interest_accrued_at = accrued_milliseconds;

        Ok(())
    }

    /// Adjusts `last_interest_accrued_at` to a weighted-average timestamp when a new borrow
    /// changes the outstanding debt mid-hour.
    ///
    /// Without adjustment, interest for the full hour would be attributed to the
    /// pre-borrow debt balance, under-charging interest on the newly borrowed amount.
    /// This computes a debt-weighted average of the previous timestamp and the current
    /// block time so that interest is fairly split between the pre-borrow and post-borrow
    /// periods.
    pub(crate) fn adjust_last_interest_accrued_at_for_new_borrow(
        &self,
        vault: &mut Vault,
        amount: Balance,
    ) -> Result<()> {
        if amount == 0 || vault.debt_balance == 0 {
            return Ok(());
        }

        let now = self.env().block_timestamp();
        let previous_debt_balance = vault.debt_balance;
        let projected_debt_balance = previous_debt_balance
            .checked_add(amount)
            .ok_or(Error::ArithmeticError)?;
        let weighted_previous = u128::from(vault.last_interest_accrued_at)
            .checked_mul(u128::from(previous_debt_balance))
            .ok_or(Error::ArithmeticError)?
            .checked_add(
                u128::from(now)
                    .checked_mul(u128::from(amount))
                    .ok_or(Error::ArithmeticError)?,
            )
            .ok_or(Error::ArithmeticError)?;
        let adjusted_timestamp = weighted_previous
            .checked_div(u128::from(projected_debt_balance))
            .ok_or(Error::ArithmeticError)?;
        if adjusted_timestamp > u64::MAX as u128 {
            return Err(Error::ArithmeticError);
        }

        #[allow(clippy::cast_possible_truncation)]
        let adjusted_timestamp = adjusted_timestamp as u64;
        vault.last_interest_accrued_at = adjusted_timestamp;

        Ok(())
    }
}
