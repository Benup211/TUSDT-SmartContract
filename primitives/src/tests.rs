#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

// Ratio fixed-point semantics tests — the trap documentation for the
// double-scaling bug class that reverted lending-pool accrual with
// `ArithmeticError` (see the `accrual_produces_interest_after_an_hour`
// regression test in tusdt-lending-pool).

use super::*;

#[test]
fn div_int_raw_integer_divisor_works() {
    // 92.54% APR → hourly rate = 0.9254 / 8760 ≈ 0.0001056393…
    let annual = Ratio::from_basis_points(9_254);
    let hourly = annual.checked_div_int(HOURS_PER_YEAR).unwrap();
    assert_eq!(hourly.into_inner(), 9_254u128 * 100_000_000_000_000 / HOURS_PER_YEAR);
    assert!(hourly > Ratio::from_inner(0));
}

#[test]
fn div_int_rejects_ratio_inner_double_scaled_divisor() {
    // THE trap: `checked_div_int` expects a RAW integer divisor. Passing a
    // Ratio's inner value (8760 × 1e18) makes the helper re-scale by another
    // 1e18 → 8.76e39 > u128::MAX → checked_from_integer overflows → None.
    // Callers that map None to ArithmeticError revert — this is exactly the
    // lending-pool accrual bug. Keep this contract: if a future change makes
    // this return Some(...), the double-scaling hazard is back.
    let annual = Ratio::from_basis_points(9_254);
    let double_scaled: u128 = Ratio::from_integer(HOURS_PER_YEAR).into_inner();
    assert_eq!(double_scaled, 8_760_000_000_000_000_000_000);
    assert_eq!(annual.checked_div_int(double_scaled), None);
}

#[test]
fn div_value_divides_value_by_ratio() {
    // checked_div_value(self, value) = value / self.
    let two = Ratio::from_integer(2);
    assert_eq!(two.checked_div_value(10).unwrap(), 5);

    // A ratio below 1.0 divides upward: 10 / 0.5 = 20.
    let half = Ratio::from_basis_points(5_000);
    assert_eq!(half.checked_div_value(10).unwrap(), 20);
}

#[test]
fn mul_value_multiplies_value_by_ratio() {
    // checked_mul_value(self, value) = value × self.
    let half = Ratio::from_basis_points(5_000);
    assert_eq!(half.checked_mul_value(10).unwrap(), 5);
    let one = Ratio::one();
    assert_eq!(one.checked_mul_value(1_000_000_000).unwrap(), 1_000_000_000);
}

#[test]
fn div_value_handles_large_u64_values() {
    // Guards the intermediate math: value_fixed × 1e18 must go through the
    // 256-bit rational multiplication, not plain u128 arithmetic — otherwise
    // any value > ~340 would overflow and None. u64::MAX / 1.0 = u64::MAX.
    let one = Ratio::one();
    assert_eq!(one.checked_div_value(u64::MAX as u128).unwrap(), u64::MAX as u128);

    // u64::MAX / 0.5 = 2 × u64::MAX — still fits u128.
    let half = Ratio::from_basis_points(5_000);
    assert_eq!(half.checked_div_value(u64::MAX as u128).unwrap(), 2 * u64::MAX as u128);
}

#[test]
fn basis_points_round_trip() {
    let ratio = Ratio::from_basis_points(1_500);
    assert_eq!(ratio.to_basis_points(), Some(1_500));
    assert_eq!(ratio.to_percentage(), Some(15));
}
