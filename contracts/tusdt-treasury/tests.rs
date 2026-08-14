#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::treasury::{
    split_delta, BUYBACK_BPS, DIVIDEND_BPS, EMERGENCY_BPS, INSURANCE_BPS, OPERATION_BPS, TOTAL_BPS,
    VOTING_BPS,
};

#[test]
fn split_zero_yields_zero_shares() {
    let shares = split_delta(0).unwrap();
    assert_eq!(shares, [0, 0, 0, 0, 0, 0]);
}

#[test]
fn split_exact_total_matches_documented_bps() {
    // 10_000 splits perfectly: emergency 5000, operation 3000, insurance 1000,
    // dividend 400, buyback 400, voting 200.
    let shares = split_delta(10_000).unwrap();
    assert_eq!(shares, [5000, 3000, 1000, 400, 400, 200]);
}

#[test]
fn split_one_million_matches_documented_bps() {
    let shares = split_delta(1_000_000).unwrap();
    assert_eq!(shares, [500_000, 300_000, 100_000, 40_000, 40_000, 20_000]);
}

#[test]
fn split_remainder_goes_to_emergency() {
    // 9999 by 5000 bps would be 4999.5 — rounds down. Remainder must land in Emergency.
    let shares = split_delta(9_999).unwrap();
    let sum: u64 = shares.iter().sum();
    assert_eq!(sum, 9_999, "shares must sum to delta");

    let op = (9_999u128 * OPERATION_BPS as u128) / TOTAL_BPS as u128;
    let ins = (9_999u128 * INSURANCE_BPS as u128) / TOTAL_BPS as u128;
    let div = (9_999u128 * DIVIDEND_BPS as u128) / TOTAL_BPS as u128;
    let buy = (9_999u128 * BUYBACK_BPS as u128) / TOTAL_BPS as u128;
    let vot = (9_999u128 * VOTING_BPS as u128) / TOTAL_BPS as u128;
    assert_eq!(shares[1] as u128, op);
    assert_eq!(shares[2] as u128, ins);
    assert_eq!(shares[3] as u128, div);
    assert_eq!(shares[4] as u128, buy);
    assert_eq!(shares[5] as u128, vot);
    // Emergency holds the documented share plus the integer-division remainder.
    let emergency_base = (9_999u128 * EMERGENCY_BPS as u128) / TOTAL_BPS as u128;
    assert!((shares[0] as u128) >= emergency_base);
}

#[test]
fn split_arbitrary_amounts_always_sum_to_delta() {
    for delta in [1u64, 7, 13, 99, 100, 12_345, 999_999, 1_234_567_890] {
        let shares = split_delta(delta).unwrap();
        let sum: u64 = shares.iter().sum();
        assert_eq!(sum, delta, "shares must sum to delta for delta={delta}");
    }
}

#[test]
fn split_handles_large_delta_without_overflow() {
    // Balance is u64. delta * 10_000 fits in u128, so any u64 delta is safe.
    let shares = split_delta(u64::MAX).unwrap();
    let sum: u128 = shares.iter().map(|&s| s as u128).sum();
    assert_eq!(sum, u64::MAX as u128);
}
