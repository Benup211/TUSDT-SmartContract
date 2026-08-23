use super::*;
use ink::codegen::Env as _;

    // Ratio arithmetic helpers (Ratio doesn't impl checked_add/checked_sub directly)
    fn ratio_add(a: Ratio, b: Ratio) -> Option<Ratio> {
        Some(Ratio::from_inner(a.into_inner().checked_add(b.into_inner())?))
    }
    fn ratio_sub(a: Ratio, b: Ratio) -> Option<Ratio> {
        Some(Ratio::from_inner(a.into_inner().checked_sub(b.into_inner())?))
    }

/// Fixed-point division helper: (num * 1e18) / denom.
/// Safe because both ≤ 1e18, so num * 1e18 ≤ 1e36 < u128::MAX (~3.4e38).
fn div_ratio(num: Ratio, denom: Ratio) -> Option<Ratio> {
    let num_inner = num.into_inner();
    let denom_inner = denom.into_inner();
    Some(Ratio::from_inner(
        num_inner.checked_mul(Ratio::one().into_inner())?.checked_div(denom_inner)?,
    ))
}

impl TusdtLendingPool {
        // ─────────────────────────────────────────────────────────────
        // Interest rate math
        // ─────────────────────────────────────────────────────────────

        /// Accrues interest for a supply/borrow market (0 = TAO, 1 = TUSDT); a
        /// no-op for alpha markets (id >= 2). Updates borrow index, exchange rate,
        /// and reserve, and emits `MarketAccrued`. Charges only whole elapsed
        /// hours and advances `last_update` by whole hours so the sub-hour
        /// remainder is preserved (vault pattern). Errors:
        /// `Error::MarketNotFound`, `Error::ArithmeticError`.
        pub(crate) fn accrue_interest(&mut self, market_id: u8) -> Result<()> {
            if market_id >= 2 {
                return Ok(());
            }
            let now = self.env().block_timestamp();
            let mut state = self.markets.get(market_id).ok_or(Error::MarketNotFound)?;
            let dt_ms = now.checked_sub(state.last_update).ok_or(Error::ArithmeticError)?;
            let dt_hours = dt_ms / tusdt_primitives::MILLISECONDS_PER_HOUR;
            if state.total_scaled_debt == 0 {
                // Nothing to accrue; keep the clock on real time. There is no
                // debt whose sub-hour remainder could be starved.
                if dt_ms > 0 {
                    state.last_update = now;
                    self.markets.insert(market_id, &state);
                }
                return Ok(());
            }
            if dt_hours == 0 {
                // Live debt but less than one full hour elapsed: leave
                // `last_update` untouched so the partial hour carries into
                // the next accrual window instead of being discarded
                // (vault pattern — whole-hours-only advance).
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
            // checked_div_int expects a RAW integer divisor, not a Ratio inner:
            // passing hours_per_year.into_inner() (8760 × 1e18) double-scales
            // and overflows u128 → ArithmeticError on every accrual.
            let hours_per_year = tusdt_primitives::HOURS_PER_YEAR;
            let borrow_rate_hourly = borrow_rate_annual
                .checked_div_int(hours_per_year)
                .ok_or(Error::ArithmeticError)?;
            let one_minus_rf =
                ratio_sub(one, params.reserve_factor).ok_or(Error::ArithmeticError)?;
            let supply_rate_annual = borrow_rate_annual
                .checked_mul(utilization)
                .and_then(|r| r.checked_mul(one_minus_rf))
                .ok_or(Error::ArithmeticError)?;
            let supply_rate_hourly = supply_rate_annual
                .checked_div_int(hours_per_year)
                .ok_or(Error::ArithmeticError)?;
            let borrow_growth = ratio_add(one, borrow_rate_hourly)
                .and_then(|f| f.checked_pow(dt_hours.into()))
                .ok_or(Error::ArithmeticError)?;
            let supply_growth = ratio_add(one, supply_rate_hourly)
                .and_then(|f| f.checked_pow(dt_hours.into()))
                .ok_or(Error::ArithmeticError)?;
            // Scaled-total accounting: the face total is derived from the
            // scaled total at the NEW borrow index. Compounding a stale floored
            // integer (the previous approach) drifted the total away from the
            // sum of per-user `scaled_debt × borrow_index` floors — the root
            // cause of unrepayable dust. The scaled total is never mutated
            // here: interest accrues purely through index growth.
            let new_borrow_index =
                state.borrow_index.checked_mul(borrow_growth).ok_or(Error::ArithmeticError)?;
            let debt_before = state.total_debt;
            let new_debt =
                scaled_debt_to_face(state.total_scaled_debt, new_borrow_index)
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
            state.borrow_index = new_borrow_index;
            state.exchange_rate = new_exchange_rate;
            state.reserve_accrued =
                state.reserve_accrued.checked_add(reserve_delta).ok_or(Error::ArithmeticError)?;
            // Advance by whole hours only (vault pattern): the sub-hour
            // remainder carries into the next accrual window instead of
            // being discarded.
            state.last_update = state
                .last_update
                .checked_add(
                    dt_hours
                        .checked_mul(tusdt_primitives::MILLISECONDS_PER_HOUR)
                        .ok_or(Error::ArithmeticError)?,
                )
                .ok_or(Error::ArithmeticError)?;
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

        /// Computes the annual borrow rate (1e18 ratio) for a given utilization
        /// from the market's interest-rate curve: `base_rate + slope1 * min(util,
        /// optimal)/optimal` plus `slope2` applied to the excess above optimal.
        /// Errors: `Error::ArithmeticError`.
        pub(crate) fn compute_borrow_rate(
            params: &InterestRateParams,
            utilization: Ratio,
        ) -> Result<Ratio> {
            if utilization.is_zero() {
                return Ok(params.base_rate);
            }
            let one = Ratio::one();
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

        /// Returns the pool's physical cash for a market: the native balance
        /// plus any TAO staked on the root subnet for TAO (market 0) and the
        /// pool's TUSDT balance for market 1. Root stake is 1:1 TAO with
        /// synchronous unstake, so it counts as cash for liquidity checks and
        /// the utilization denominator. Errors: `Error::MarketNotFound` for
        /// any other market, `Error::ArithmeticError` on overflow.
        pub(crate) fn market_cash(&self, market_id: u8) -> Result<Balance> {
            match market_id {
                0 => self
                    .env()
                    .balance()
                    .checked_add(self.staked_tao)
                    .ok_or(Error::ArithmeticError),
                1 => Ok(self.tusdt.balance_of(self.env().account_id())),
                _ => Err(Error::MarketNotFound),
            }
        }
}
