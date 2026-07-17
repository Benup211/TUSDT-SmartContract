#![cfg_attr(not(feature = "std"), no_std)]
//! Shared primitive types for the tUSDT protocol. Provides the [`Ratio`] fixed-point arithmetic
//! type (wrapping `FixedU128`) for basis-point and percentage calculations, together with
//! exponential / power functions for the vault's discrete hourly compounding model, and a set of
//! time-constant definitions used throughout the protocol.

use sp_arithmetic::fixed_point::FixedPointNumber;
use sp_arithmetic::traits::{CheckedAdd, CheckedDiv, CheckedMul, One, Zero};
use sp_arithmetic::FixedU128;

/// Milliseconds in one second.
pub const MILLISECONDS_PER_SECOND: u64 = 1_000;
/// Seconds in one hour (3,600).
pub const SECONDS_PER_HOUR: u64 = 3_600;
/// Hours in one day (24).
pub const HOURS_PER_DAY: u64 = 24;
/// Seconds in one day (86,400).
pub const SECONDS_PER_DAY: u64 = 86_400;
/// Milliseconds in one hour.
pub const MILLISECONDS_PER_HOUR: u64 = SECONDS_PER_HOUR * MILLISECONDS_PER_SECOND;
/// Milliseconds in one day.
pub const MILLISECONDS_PER_DAY: u64 = SECONDS_PER_DAY * MILLISECONDS_PER_SECOND;
/// Days in a standard year (365); used in APR↔hourly rate conversion.
pub const DAYS_PER_YEAR: u128 = 365;
/// Hours in a standard year (365 × 24 = 8,760); the compounding period count for the annual interest rate.
pub const HOURS_PER_YEAR: u128 = DAYS_PER_YEAR * HOURS_PER_DAY as u128;
const FIXED_SCALE: u128 = <FixedU128 as FixedPointNumber>::DIV;
const PERCENT_DENOMINATOR: u128 = 100;
const BASIS_POINTS_DENOMINATOR: u128 = 10_000;

/// A fixed-point ratio wrapping `FixedU128` for on-chain arithmetic. Supports basis-point and
/// percentage conversion, checked multiplication/division, and exponential/power operations for
/// the vault's hourly compounding model.
#[ink::scale_derive(Encode, Decode, TypeInfo)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
pub struct Ratio(u128);

impl Ratio {
    /// Creates a `Ratio` from the raw `u128` inner value of a `FixedU128`. The
    /// 18-decimal-place fixed-point scale means `from_inner(1_000_000_000_000_000_000)` is
    /// equivalent to `from_integer(1)`. **Prefer [`from_integer`] for whole-number ratios**
    /// — a raw inner of `1` represents ~1e-18, not 1.0.
    pub const fn from_inner(inner: u128) -> Self {
        Self(inner)
    }

    /// Creates a `Ratio` from a `FixedU128` value.
    pub fn from_fixed(value: FixedU128) -> Self {
        Self(value.into_inner())
    }

    /// Creates a `Ratio` from a `u128` integer (maps to value × `FixedU128::DIV`).
    pub fn from_integer(value: u128) -> Self {
        Self(
            FixedU128::checked_from_integer(value)
                .expect("integer ratio should fit in fixed-point representation")
                .into_inner(),
        )
    }

    /// Creates a `Ratio` from a percentage (p / 100).
    pub fn from_percentage(percent: u32) -> Self {
        Self(from_percentage(percent).into_inner())
    }

    /// Converts this `Ratio` to a percentage integer (self × 100). Returns `None` if the result exceeds the `u32` range.
    pub fn to_percentage(self) -> Option<u32> {
        let percent = self.as_fixed().checked_mul_int(PERCENT_DENOMINATOR)?;
        u32::try_from(percent).ok()
    }

    /// Creates a `Ratio` from basis points (bp / 10,000).
    pub fn from_basis_points(basis_points: u32) -> Self {
        Self(from_basis_points(basis_points).into_inner())
    }

    /// Converts this `Ratio` to basis points (self × 10,000). Returns `None` if the result exceeds the `u32` range.
    pub fn to_basis_points(self) -> Option<u32> {
        let basis_points = self.as_fixed().checked_mul_int(BASIS_POINTS_DENOMINATOR)?;
        u32::try_from(basis_points).ok()
    }

    /// Returns the raw `FixedU128` inner value.
    pub const fn into_inner(self) -> u128 {
        self.0
    }

    /// Returns the `Ratio` representing 1.0 (100%).
    pub const fn one() -> Self {
        Self(FIXED_SCALE)
    }

    /// Converts this `Ratio` to a `FixedU128`.
    pub fn as_fixed(self) -> FixedU128 {
        FixedU128::from_inner(self.0)
    }

    /// Returns `true` if the `Ratio` is zero.
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Multiplies a `u128` value by this `Ratio`. Returns `None` on overflow.
    pub fn checked_mul_value(self, value: u128) -> Option<u128> {
        self.as_fixed().checked_mul_int(value)
    }

    /// Multiplies this `Ratio` by another `Ratio`. Returns `None` on overflow.
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        self.as_fixed()
            .checked_mul(&rhs.as_fixed())
            .map(Self::from_fixed)
    }

    /// Returns the absolute difference between two `Ratio`s.
    pub fn abs_diff(self, other: Self) -> Self {
        Self(self.0.abs_diff(other.0))
    }

    /// Divides a `u128` value by this `Ratio` (`value / self`). Returns `None` on overflow or division by zero.
    pub fn checked_div_value(self, value: u128) -> Option<u128> {
        let value_fixed = FixedU128::checked_from_integer(value)?;
        value_fixed
            .checked_div(&self.as_fixed())?
            .checked_mul_int(1_u128)
    }

    /// Divides this `Ratio` by a `u128` integer (`self / rhs`). Returns `None` on overflow.
    pub fn checked_div_int(self, rhs: u128) -> Option<Self> {
        let rhs_fixed = FixedU128::checked_from_integer(rhs)?;
        self.as_fixed()
            .checked_div(&rhs_fixed)
            .map(Self::from_fixed)
    }

    /// Computes e^`self` using a 32-term Taylor series approximation. Returns `None` on overflow.
    pub fn exp(self) -> Option<Self> {
        exp_fixed(self.as_fixed()).map(Self::from_fixed)
    }

    /// Raises this `Ratio` to an integer power using the square-and-multiply algorithm. Returns `None` on overflow.
    pub fn checked_pow(self, exponent: u128) -> Option<Self> {
        pow_fixed(self.as_fixed(), exponent).map(Self::from_fixed)
    }
}

/// Creates a `FixedU128` ratio from a percentage (p / 100).
pub fn from_percentage(percent: u32) -> FixedU128 {
    FixedU128::saturating_from_rational(percent as u128, PERCENT_DENOMINATOR)
}

/// Creates a `FixedU128` ratio from basis points (bp / 10,000).
pub fn from_basis_points(basis_points: u32) -> FixedU128 {
    FixedU128::saturating_from_rational(basis_points as u128, BASIS_POINTS_DENOMINATOR)
}

/// Computes e^`exponent` via a 32-term Taylor series. Returns `None` on overflow.
pub fn exp_fixed(exponent: FixedU128) -> Option<FixedU128> {
    let mut sum = FixedU128::one();
    let mut term = FixedU128::one();

    for n in 1..=32_u128 {
        let next_term = term.checked_mul(&exponent)?;
        let n_fixed = FixedU128::checked_from_integer(n)?;
        term = next_term.checked_div(&n_fixed)?;
        if term.is_zero() {
            break;
        }
        sum = sum.checked_add(&term)?;
    }

    Some(sum)
}

/// Computes `base`^`exponent` via the square-and-multiply algorithm. Returns `None` on overflow.
pub fn pow_fixed(mut base: FixedU128, mut exponent: u128) -> Option<FixedU128> {
    let mut result = FixedU128::one();

    while exponent > 0 {
        if exponent % 2 == 1 {
            result = result.checked_mul(&base)?;
        }
        exponent /= 2;
        if exponent > 0 {
            base = base.checked_mul(&base)?;
        }
    }

    Some(result)
}
