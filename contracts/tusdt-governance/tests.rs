// Tests use ergonomic plain arithmetic on bounded constants; the strict numeric lints aren't
// useful here and would just clutter assertions.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::redundant_pattern_matching
)]

use super::governance::*;
use ink::prelude::string::String;
use ink::prelude::vec::Vec;
use tusdt_env::StakeInfo;
use tusdt_treasury::{Fund, TokenKind};

/// Default mocked stake (in base units) returned for the proposer, comfortably above the
/// default `min_proposer_stake` floor so submission is allowed unless a test overrides it.
const STAKE_ABOVE_FLOOR: u64 = 2_000_000_000_000;

/// Off-chain mock of the `get_stake_info_for_hotkey_coldkey_netuid` chain extension.
/// `stake = Some(v)` resolves to a `StakeInfo` with that alpha stake; `None` mimics a pair
/// with no stake record.
struct StakeExtension {
    stake: Option<u64>,
}

impl ink::env::test::ChainExtension for StakeExtension {
    fn ext_id(&self) -> u16 {
        0x1000
    }

    fn call(&mut self, _func_id: u16, _input: &[u8], output: &mut Vec<u8>) -> u32 {
        let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
        let info = self.stake.map(|stake| StakeInfo {
            hotkey: accounts.alice,
            coldkey: accounts.alice,
            netuid: ink::scale::Compact(113),
            stake: ink::scale::Compact(stake),
            locked: ink::scale::Compact(0),
            emission: ink::scale::Compact(0),
            tao_emission: ink::scale::Compact(0),
            drain: ink::scale::Compact(0),
            is_registered: true,
        });
        ink::scale::Encode::encode_to(&info, output);
        0
    }
}

fn register_stake(stake: Option<u64>) {
    ink::env::test::register_chain_extension(StakeExtension { stake });
}

const MS_PER_DAY: u64 = 86_400_000;

/// Builds a Unix-epoch timestamp (ms) for the `day`-th of the month. Epoch day 0 is
/// 1970-01-01 (the 1st), so `day` maps to `(day - 1)` whole days past the epoch.
fn ts_for_day(day: u64) -> u64 {
    (day - 1) * MS_PER_DAY
}

/// A timestamp comfortably inside the default 5..=25 submission window (the 15th).
const IN_WINDOW_TS: u64 = 14 * MS_PER_DAY;

fn set_block_timestamp(ts: u64) {
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(ts);
}

fn set_caller(caller: ink::primitives::AccountId) {
    let callee = ink::env::account_id::<tusdt_env::CustomEnvironment>();
    ink::env::test::set_callee::<tusdt_env::CustomEnvironment>(callee);
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(caller);
}

/// Constructs governance with the test clock/stake set, but no snapshot submitted yet.
fn make_governance_no_snapshot() -> TusdtGovernance {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    // Default the proposer's stake above the floor; individual tests can re-register.
    register_stake(Some(STAKE_ABOVE_FLOOR));
    // Default the clock inside the submission window; individual tests can override.
    set_block_timestamp(IN_WINDOW_TS);
    // Maintainer is alice (the caller); the treasury address is a stand-in account.
    let mut gov = TusdtGovernance::new(accounts.django, accounts.alice);
    // Seat a full council including alice so operational duties (submit_snapshot) work under the
    // default caller. frank is intentionally left out as a non-council account for negative tests.
    gov.set_council(ink::prelude::vec![
        accounts.alice,
        accounts.bob,
        accounts.charlie,
        accounts.django,
        accounts.eve,
    ])
    .expect("set council");
    gov
}

/// Constructs governance and commits an initial empty snapshot (epoch 1, zero root, zero supply)
/// so proposals can be submitted. Voting tests submit their own snapshot with a real root.
fn make_governance() -> TusdtGovernance {
    let mut gov = make_governance_no_snapshot();
    gov.submit_snapshot([0u8; 32], 0, 0).expect("snapshot ok");
    gov
}

/// Commits a snapshot whose Merkle tree holds exactly one leaf for `(coldkey, hotkey, balance,
/// multiplier_bps)`. For a single-leaf tree the root is the leaf and the proof is empty. Returns
/// the new epoch.
fn submit_single_leaf_snapshot(
    gov: &mut TusdtGovernance,
    coldkey: ink::primitives::AccountId,
    hotkey: ink::primitives::AccountId,
    balance: u128,
    multiplier_bps: u32,
    circulating_supply: u128,
) -> u64 {
    let root = leaf_hash(coldkey, hotkey, balance, multiplier_bps);
    gov.submit_snapshot(root, circulating_supply, 0)
        .expect("snapshot ok")
}

#[ink::test]
fn constructor_sets_maintainer_and_defaults() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let gov = make_governance_no_snapshot();
    assert_eq!(gov.maintainer(), accounts.alice);
    // The helper seats a full council; alice is a member and frank is not.
    assert_eq!(gov.council().len(), 5);
    assert!(gov.is_council(accounts.alice));
    assert!(!gov.is_council(accounts.frank));
    assert_eq!(gov.proposal_count(), 0);
    let p = gov.params();
    assert_eq!(p.netuid, 113);
    assert_eq!(p.voting_period_ms, 48 * 60 * 60 * 1_000);
    assert_eq!(p.approval_bps, 5_001);
    assert_eq!(p.quorum_bps, 2_000);
    assert_eq!(p.min_proposer_stake, 1_000_000_000_000);
    assert_eq!(p.submission_open_day, 5);
    assert_eq!(p.submission_close_day, 25);
    // No snapshot yet → no epoch, no snapshot record, quorum threshold of zero.
    assert_eq!(gov.current_epoch(), 0);
    assert!(gov.get_snapshot(1).is_none());
    assert_eq!(gov.quorum(1), 0);
}

#[ink::test]
fn submit_proposal_rejects_without_snapshot() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance_no_snapshot();
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob);
    assert_eq!(res, Err(Error::NoSnapshot));
}

#[ink::test]
fn submit_proposal_rejects_empty_cid() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let res = gov.submit_proposal(String::new(), ProposalKind::NonFunding, accounts.bob);
    assert!(matches!(res, Err(_)));
}

#[ink::test]
fn submit_proposal_rejects_oversized_cid() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let long_cid = String::from_utf8(vec![b'a'; 97]).unwrap();
    let res = gov.submit_proposal(long_cid, ProposalKind::NonFunding, accounts.bob);
    assert!(matches!(res, Err(_)));
}

#[ink::test]
fn submit_proposal_rejects_zero_amount_funding() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let res = gov.submit_proposal(
        String::from("bafy..."),
        ProposalKind::Funding {
            fund: Fund::Operation,
            token_kind: TokenKind::Tusdt,
            amount: 0,
            recipient: accounts.bob,
        },
        accounts.bob,
    );
    assert!(matches!(res, Err(_)));
}

#[ink::test]
fn submit_proposal_happy_path_increments_count() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let id = gov
        .submit_proposal(
            String::from("bafy-cid-1"),
            ProposalKind::NonFunding,
            accounts.bob,
        )
        .expect("submit ok");
    assert_eq!(id, 1);
    assert_eq!(gov.proposal_count(), 1);
    let p = gov.get_proposal(1).expect("proposal stored");
    assert_eq!(p.id, 1);
    assert_eq!(p.status, ProposalStatus::Active);
    assert_eq!(p.yes, 0);
    assert_eq!(p.no, 0);

    let id2 = gov
        .submit_proposal(
            String::from("bafy-cid-2"),
            ProposalKind::NonFunding,
            accounts.bob,
        )
        .expect("submit ok");
    assert_eq!(id2, 2);
    assert_eq!(gov.proposal_count(), 2);
}

/// Sets the proposer-stake floor on `gov` to `floor`, leaving other params at their defaults.
fn set_proposer_stake_floor(gov: &mut TusdtGovernance, floor: u128) {
    let mut p = GovernanceParams::default_params();
    p.min_proposer_stake = floor;
    gov.update_params(p).expect("update ok");
}

#[ink::test]
fn submit_proposal_rejects_stake_at_or_below_floor() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    set_proposer_stake_floor(&mut gov, 1_000);

    // Exactly at the floor is not "greater than", so it must be rejected.
    register_stake(Some(1_000));
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob);
    assert_eq!(res, Err(Error::InsufficientStake));

    // Below the floor is likewise rejected.
    register_stake(Some(999));
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob);
    assert_eq!(res, Err(Error::InsufficientStake));

    assert_eq!(gov.proposal_count(), 0);
}

#[ink::test]
fn submit_proposal_rejects_when_no_stake_record() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    register_stake(None);
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob);
    assert_eq!(res, Err(Error::NoStake));
    assert_eq!(gov.proposal_count(), 0);
}

#[ink::test]
fn submit_proposal_allows_stake_just_above_floor() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    set_proposer_stake_floor(&mut gov, 1_000);

    register_stake(Some(1_001));
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("submit ok");
    assert_eq!(id, 1);
    assert_eq!(gov.proposal_count(), 1);
}

#[ink::test]
fn update_params_changes_proposer_stake_floor() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // A 5k-unit stake is below the default 1e12 floor → rejected.
    register_stake(Some(5_000));
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob);
    assert_eq!(res, Err(Error::InsufficientStake));

    // Lower the floor below that stake and the same submission now succeeds.
    set_proposer_stake_floor(&mut gov, 1_000);
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("submit ok");
    assert_eq!(id, 1);
}

#[ink::test]
fn submit_proposal_rejects_before_window_opens() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // The 4th is before the default open day (5th).
    set_block_timestamp(ts_for_day(4));
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob);
    assert_eq!(res, Err(Error::OutsideSubmissionWindow));
    assert_eq!(gov.proposal_count(), 0);
}

#[ink::test]
fn submit_proposal_rejects_after_window_closes() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // The 26th is after the default close day (25th).
    set_block_timestamp(ts_for_day(26));
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob);
    assert_eq!(res, Err(Error::OutsideSubmissionWindow));
    assert_eq!(gov.proposal_count(), 0);
}

#[ink::test]
fn submit_proposal_allows_on_window_boundaries() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // Both the open (5th) and close (25th) days are inclusive.
    set_block_timestamp(ts_for_day(5));
    gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("5th accepted");

    set_block_timestamp(ts_for_day(25));
    gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("25th accepted");

    assert_eq!(gov.proposal_count(), 2);
}

#[ink::test]
fn update_params_rejects_invalid_submission_window() {
    let mut gov = make_governance();

    // open_day < 1
    let mut bad = GovernanceParams::default_params();
    bad.submission_open_day = 0;
    assert_eq!(gov.update_params(bad), Err(Error::InvalidParams));

    // open_day > close_day
    let mut bad = GovernanceParams::default_params();
    bad.submission_open_day = 20;
    bad.submission_close_day = 10;
    assert_eq!(gov.update_params(bad), Err(Error::InvalidParams));

    // close_day > 28 (not reachable every month)
    let mut bad = GovernanceParams::default_params();
    bad.submission_close_day = 29;
    assert_eq!(gov.update_params(bad), Err(Error::InvalidParams));
}

#[ink::test]
fn submit_proposal_respects_updated_window() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // Narrow the window to the 10th–12th; the 15th (default clock) now falls outside it.
    let mut p = GovernanceParams::default_params();
    p.submission_open_day = 10;
    p.submission_close_day = 12;
    gov.update_params(p).expect("update ok");

    set_block_timestamp(ts_for_day(15));
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob);
    assert_eq!(res, Err(Error::OutsideSubmissionWindow));

    set_block_timestamp(ts_for_day(11));
    gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("11th accepted");
    assert_eq!(gov.proposal_count(), 1);
}

#[ink::test]
fn day_of_month_matches_known_dates() {
    // 1970-01-01 (epoch) is the 1st.
    assert_eq!(day_of_month(0), 1);
    // 2021-01-01 00:00:00 UTC.
    assert_eq!(day_of_month(1_609_459_200_000), 1);
    // 2020-02-29 00:00:00 UTC (leap day).
    assert_eq!(day_of_month(1_582_934_400_000), 29);
    // 2024-12-31 00:00:00 UTC.
    assert_eq!(day_of_month(1_735_603_200_000), 31);
    // Mid-day on the 15th should still report the 15th.
    assert_eq!(day_of_month(ts_for_day(15) + 12 * 60 * 60 * 1_000), 15);
}

#[ink::test]
fn integer_sqrt_floors_correctly() {
    assert_eq!(integer_sqrt(0), 0);
    assert_eq!(integer_sqrt(1), 1);
    assert_eq!(integer_sqrt(2), 1);
    assert_eq!(integer_sqrt(3), 1);
    assert_eq!(integer_sqrt(4), 2);
    assert_eq!(integer_sqrt(8), 2);
    assert_eq!(integer_sqrt(9), 3);
    assert_eq!(integer_sqrt(1_000_000), 1_000);
    assert_eq!(integer_sqrt(4_000_000_000_000), 2_000_000);
    // Largest perfect square below u128::MAX round-trips.
    let big = (u64::MAX as u128) * (u64::MAX as u128);
    assert_eq!(integer_sqrt(big), u64::MAX as u128);
}

#[ink::test]
fn vote_weight_is_sqrt_of_balance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // Single-leaf snapshot: balance 4e12, 1.0x multiplier. sqrt(4e12) = 2_000_000.
    let balance = 4_000_000_000_000;
    submit_single_leaf_snapshot(&mut gov, accounts.alice, accounts.bob, balance, 10_000, 0);
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("submit ok");

    gov.vote(id, accounts.bob, true, balance, 10_000, Vec::new())
        .expect("vote ok");

    let proposal = gov.get_proposal(id).unwrap();
    // With the 1.0x time-staked multiplier, weight == sqrt(balance).
    assert_eq!(proposal.yes, 2_000_000);
    assert_eq!(proposal.no, 0);
}

#[ink::test]
fn vote_weight_applies_time_staked_multiplier() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // sqrt(4e12) = 2_000_000; 0.8x multiplier → 1_600_000.
    let balance = 4_000_000_000_000;
    submit_single_leaf_snapshot(&mut gov, accounts.alice, accounts.bob, balance, 8_000, 0);
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("submit ok");

    gov.vote(id, accounts.bob, true, balance, 8_000, Vec::new())
        .expect("vote ok");

    assert_eq!(gov.get_proposal(id).unwrap().yes, 1_600_000);
}

#[ink::test]
fn vote_rejects_invalid_proof() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    let balance = 4_000_000_000_000;
    submit_single_leaf_snapshot(&mut gov, accounts.alice, accounts.bob, balance, 10_000, 0);
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("submit ok");

    // A balance that doesn't match the committed leaf can't be proven.
    let res = gov.vote(id, accounts.bob, true, balance * 2, 10_000, Vec::new());
    assert_eq!(res, Err(Error::InvalidProof));
    assert_eq!(gov.get_proposal(id).unwrap().yes, 0);
}

#[ink::test]
fn vote_verifies_multi_leaf_proof() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // Two-leaf tree; the voter (alice/bob) proves membership with the sibling leaf.
    let balance = 4_000_000_000_000;
    let leaf_voter = leaf_hash(accounts.alice, accounts.bob, balance, 10_000);
    let leaf_other = leaf_hash(accounts.charlie, accounts.eve, 1_000_000, 10_000);
    let root = hash_pair(leaf_voter, leaf_other);
    gov.submit_snapshot(root, 0, 0).expect("snapshot ok");

    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("submit ok");

    gov.vote(id, accounts.bob, true, balance, 10_000, vec![leaf_other])
        .expect("vote ok");
    assert_eq!(gov.get_proposal(id).unwrap().yes, 2_000_000);
}

#[ink::test]
fn vote_rejects_double_vote_for_same_pair() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    let balance = 4_000_000_000_000;
    submit_single_leaf_snapshot(&mut gov, accounts.alice, accounts.bob, balance, 10_000, 0);
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("submit ok");

    gov.vote(id, accounts.bob, true, balance, 10_000, Vec::new())
        .expect("vote ok");
    assert_eq!(
        gov.vote(id, accounts.bob, false, balance, 10_000, Vec::new()),
        Err(Error::AlreadyVoted)
    );
}

#[ink::test]
fn update_params_rejects_non_maintainer() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    // A council member is still not the maintainer and cannot update params.
    set_caller(accounts.bob);
    assert_eq!(
        gov.update_params(GovernanceParams::default_params()),
        Err(Error::NotMaintainer)
    );
}

#[ink::test]
fn update_params_rejects_invalid_approval_bps() {
    let mut gov = make_governance();
    let mut bad = GovernanceParams::default_params();
    bad.approval_bps = 0;
    assert!(gov.update_params(bad).is_err());

    let mut bad = GovernanceParams::default_params();
    bad.approval_bps = 10_001;
    assert!(gov.update_params(bad).is_err());
}

#[ink::test]
fn update_params_rejects_zero_voting_period() {
    let mut gov = make_governance();
    let mut bad = GovernanceParams::default_params();
    bad.voting_period_ms = 0;
    assert!(gov.update_params(bad).is_err());
}

#[ink::test]
fn update_params_happy_path() {
    let mut gov = make_governance();
    let mut p = GovernanceParams::default_params();
    p.netuid = 42;
    p.quorum_bps = 3_000;
    p.approval_bps = 6_667;
    p.voting_period_ms = 60 * 1_000;
    gov.update_params(p).expect("update ok");
    assert_eq!(gov.params().netuid, 42);
    assert_eq!(gov.params().quorum_bps, 3_000);
    assert_eq!(gov.params().approval_bps, 6_667);
    assert_eq!(gov.params().voting_period_ms, 60 * 1_000);
}

#[ink::test]
fn finalize_before_voting_ends_fails() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .unwrap();
    assert!(gov.finalize(id).is_err());
}

#[ink::test]
fn execute_on_non_passed_fails() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .unwrap();
    // Still Active → cannot execute.
    assert!(gov.execute(id).is_err());
}

#[ink::test]
fn finalize_rejects_when_quorum_not_met() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let mut p = GovernanceParams::default_params();
    p.voting_period_ms = 1; // expire quickly
    gov.update_params(p).expect("update ok");
    // A non-zero circulating supply yields a non-zero quorum (20% of 100 = 20), so a
    // zero-vote proposal falls short and is rejected on finalize.
    let epoch = gov.submit_snapshot([0u8; 32], 100, 0).expect("snapshot ok");
    assert_eq!(gov.quorum(epoch), 20);

    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .unwrap();

    // Advance block timestamp past voting_ends_at.
    ink::env::test::advance_block::<tusdt_env::CustomEnvironment>();
    ink::env::test::advance_block::<tusdt_env::CustomEnvironment>();

    gov.finalize(id).expect("finalize ok");
    let proposal = gov.get_proposal(id).unwrap();
    assert_eq!(proposal.status, ProposalStatus::Rejected);
}

#[ink::test]
fn quorum_is_20_percent_of_circulating_supply_by_default() {
    let mut gov = make_governance();

    let epoch = gov
        .submit_snapshot([0u8; 32], 1_000_000, 0)
        .expect("snapshot ok");
    assert_eq!(
        gov.get_snapshot(epoch).unwrap().circulating_supply,
        1_000_000
    );
    // 20% of 1_000_000.
    assert_eq!(gov.quorum(epoch), 200_000);
}

#[ink::test]
fn submit_snapshot_rejects_non_council() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let epoch_before = gov.current_epoch();
    // frank is not on the council and cannot commit a snapshot.
    set_caller(accounts.frank);
    assert_eq!(
        gov.submit_snapshot([0u8; 32], 1_000_000, 0),
        Err(Error::NotCouncil)
    );
    assert_eq!(gov.current_epoch(), epoch_before);
}

#[ink::test]
fn update_params_rejects_invalid_quorum_bps() {
    let mut gov = make_governance();
    let mut bad = GovernanceParams::default_params();
    bad.quorum_bps = 10_001;
    assert_eq!(gov.update_params(bad), Err(Error::InvalidParams));
}

#[ink::test]
fn finalize_passes_when_quorum_met() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let mut p = GovernanceParams::default_params();
    p.voting_period_ms = 1; // expire quickly
    gov.update_params(p).expect("update ok");

    // Supply 1e6 → quorum 200_000; the single voter's 4e12 balance clears it comfortably.
    let balance = 4_000_000_000_000;
    let epoch = submit_single_leaf_snapshot(
        &mut gov,
        accounts.alice,
        accounts.bob,
        balance,
        10_000,
        1_000_000,
    );
    assert!(gov.quorum(epoch) < balance);

    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .unwrap();
    gov.vote(id, accounts.bob, true, balance, 10_000, Vec::new())
        .expect("vote ok");

    ink::env::test::advance_block::<tusdt_env::CustomEnvironment>();
    ink::env::test::advance_block::<tusdt_env::CustomEnvironment>();

    gov.finalize(id).expect("finalize ok");
    let proposal = gov.get_proposal(id).unwrap();
    assert_eq!(proposal.status, ProposalStatus::Passed);
    // Quorum is tracked in raw balance, not voting power.
    assert_eq!(proposal.voted_balance, balance);
}

#[ink::test]
fn finalize_rejects_when_voted_balance_below_quorum() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let mut p = GovernanceParams::default_params();
    p.voting_period_ms = 1; // expire quickly
    gov.update_params(p).expect("update ok");

    // Voter's balance (1e6) is below the quorum (20% of 1e8 = 2e7), so a unanimous yes still
    // fails quorum — the threshold is measured in raw balance, not voting power.
    let balance = 1_000_000;
    let epoch = submit_single_leaf_snapshot(
        &mut gov,
        accounts.alice,
        accounts.bob,
        balance,
        10_000,
        100_000_000,
    );
    assert_eq!(gov.quorum(epoch), 20_000_000);

    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .unwrap();
    gov.vote(id, accounts.bob, true, balance, 10_000, Vec::new())
        .expect("vote ok");

    ink::env::test::advance_block::<tusdt_env::CustomEnvironment>();
    ink::env::test::advance_block::<tusdt_env::CustomEnvironment>();

    gov.finalize(id).expect("finalize ok");
    let proposal = gov.get_proposal(id).unwrap();
    assert_eq!(proposal.status, ProposalStatus::Rejected);
    assert_eq!(proposal.voted_balance, balance);
}

#[ink::test]
fn set_maintainer_transfers_role() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    gov.set_maintainer(accounts.bob).expect("transfer ok");
    assert_eq!(gov.maintainer(), accounts.bob);

    // The old maintainer (alice) can no longer govern; the new one can.
    set_caller(accounts.alice);
    assert_eq!(
        gov.update_params(GovernanceParams::default_params()),
        Err(Error::NotMaintainer)
    );
    set_caller(accounts.bob);
    gov.update_params(GovernanceParams::default_params())
        .expect("update ok");
}

#[ink::test]
fn set_maintainer_rejects_non_maintainer() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    set_caller(accounts.bob);
    assert_eq!(
        gov.set_maintainer(accounts.bob),
        Err(Error::NotMaintainer)
    );
    assert_eq!(gov.maintainer(), accounts.alice);
}

#[ink::test]
fn set_council_replaces_membership() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    gov.set_council(ink::prelude::vec![
        accounts.bob,
        accounts.charlie,
        accounts.django,
        accounts.eve,
        accounts.frank,
    ])
    .expect("set council ok");
    // alice was dropped; frank was added.
    assert!(!gov.is_council(accounts.alice));
    assert!(gov.is_council(accounts.frank));
    assert_eq!(gov.council().len(), 5);
}

#[ink::test]
fn set_council_rejects_non_maintainer() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    // A sitting council member cannot reseat the council; only the maintainer can.
    set_caller(accounts.bob);
    assert_eq!(
        gov.set_council(ink::prelude::vec![
            accounts.bob,
            accounts.charlie,
            accounts.django,
            accounts.eve,
            accounts.frank,
        ]),
        Err(Error::NotMaintainer)
    );
}

#[ink::test]
fn set_council_rejects_wrong_size() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    assert_eq!(
        gov.set_council(ink::prelude::vec![accounts.bob, accounts.charlie]),
        Err(Error::InvalidCouncil)
    );
}

#[ink::test]
fn set_council_rejects_duplicates() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    assert_eq!(
        gov.set_council(ink::prelude::vec![
            accounts.bob,
            accounts.bob,
            accounts.charlie,
            accounts.django,
            accounts.eve,
        ]),
        Err(Error::InvalidCouncil)
    );
}
