use super::*;

/// Returns the UTC day-of-month (1..=31) for a Unix-epoch timestamp in milliseconds.
///
/// Uses Howard Hinnant's `civil_from_days` algorithm. Timestamps are always post-1970, so the
/// day count is non-negative and the negative-era branch of the general algorithm is omitted —
/// all arithmetic stays in u64. The constants and ranges are well-known and bounded
/// (e.g. `doy <= 365`, `mp <= 11`, `day <= 31`), so `as u8` is safe.
//
// The Hinnant algorithm relies on standard `+`/`-`/`*`/`/` and is provably non-overflowing for
// any u64 millisecond timestamp; rewriting it with `checked_*` everywhere would obscure the
// structure without adding safety.
#[allow(clippy::arithmetic_side_effects, clippy::cast_possible_truncation)]
pub(crate) fn day_of_month(timestamp_ms: u64) -> u8 {
    // Days since the 1970-01-01 epoch.
    let days: u64 = timestamp_ms / MS_PER_DAY;
    // Shift the era origin to 0000-03-01 so leap days fall at the end of the 400-year cycle.
    let z: u64 = days + 719_468;
    let era: u64 = z / 146_097;
    let doe: u64 = z - era * 146_097; // day-of-era, [0, 146096]
    let yoe: u64 = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year-of-era, [0, 399]
    let doy: u64 = doe - (365 * yoe + yoe / 4 - yoe / 100); // day-of-year (Mar-based), [0, 365]
    let mp: u64 = (5 * doy + 2) / 153; // month, Mar=0..Feb=11
    let day: u64 = doy - (153 * mp + 2) / 5 + 1; // day-of-month, [1, 31]
    day as u8
}

/// Integer (floor) square root of `n`, via Newton's method. No floats are available on-chain.
//
// Newton's iteration is monotonically decreasing and bounded; no operation can overflow for
// any u128 input. Annotating with checked_* every step would mask the standard form.
#[allow(clippy::arithmetic_side_effects)]
pub(crate) fn integer_sqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Voting power for a snapshot leaf: `sqrt(balance) * multiplier_bps / 10_000`.
/// The square root dampens large stakes; the time-staked `multiplier_bps` rewards longer-held
/// stake (0.5x = 5_000, 0.8x = 8_000, 1.0x = 10_000).
pub(crate) fn voting_power(balance: u128, multiplier_bps: u32) -> Result<u128> {
    integer_sqrt(balance)
        .checked_mul(u128::from(multiplier_bps))
        .and_then(|scaled| scaled.checked_div(u128::from(BPS_DENOMINATOR)))
        .ok_or(Error::ArithmeticError)
}

/// Computes a snapshot leaf hash: `blake2_256(SCALE(coldkey, hotkey, balance, multiplier_bps))`.
/// Off-chain proof generation must encode and hash the tuple identically.
pub(crate) fn leaf_hash(
    coldkey: AccountId,
    hotkey: AccountId,
    balance: u128,
    multiplier_bps: u32,
) -> MerkleHash {
    let mut out = MerkleHash::default();
    ink::env::hash_encoded::<ink::env::hash::Blake2x256, _>(
        &(coldkey, hotkey, balance, multiplier_bps),
        &mut out,
    );
    out
}

/// Hashes a pair of sibling nodes in sorted order: `blake2_256(min(a,b) ++ max(a,b))`.
/// Sorting lets proofs omit per-node position flags (OpenZeppelin convention).
pub(crate) fn hash_pair(a: MerkleHash, b: MerkleHash) -> MerkleHash {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let mut input = [0u8; 64];
    input[..32].copy_from_slice(&lo);
    input[32..].copy_from_slice(&hi);
    let mut out = MerkleHash::default();
    ink::env::hash_bytes::<ink::env::hash::Blake2x256>(&input, &mut out);
    out
}

/// Verifies that `leaf` is part of the tree with the given `root`, folding `proof` bottom-up.
pub(crate) fn verify_merkle_proof(
    proof: &[MerkleHash],
    root: MerkleHash,
    leaf: MerkleHash,
) -> bool {
    let mut computed = leaf;
    for sibling in proof {
        computed = hash_pair(computed, *sibling);
    }
    computed == root
}
