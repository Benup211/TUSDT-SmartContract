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

fn make_governance() -> TusdtGovernance {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    // Default the proposer's stake above the floor; individual tests can re-register.
    register_stake(Some(STAKE_ABOVE_FLOOR));
    // Default the clock inside the submission window; individual tests can override.
    set_block_timestamp(IN_WINDOW_TS);
    TusdtGovernance::new(accounts.django)
}

#[ink::test]
fn constructor_sets_admin_and_defaults() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let gov = make_governance();
    assert_eq!(gov.admin(), accounts.alice);
    assert_eq!(gov.proposal_count(), 0);
    let p = gov.params();
    assert_eq!(p.netuid, 113);
    assert_eq!(p.voting_period_ms, 48 * 60 * 60 * 1_000);
    assert_eq!(p.approval_bps, 5_001);
    assert_eq!(p.quorum_bps, 2_000);
    assert_eq!(p.min_proposer_stake, 1_000_000_000_000);
    assert_eq!(p.submission_open_day, 5);
    assert_eq!(p.submission_close_day, 25);
    // No snapshot yet → no circulating supply → quorum threshold of zero.
    assert_eq!(gov.circulating_supply(), 0);
    assert_eq!(gov.quorum(), 0);
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
fn vote_weight_is_sqrt_of_stake() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // 4e12 raw stake clears the 1e12 proposer floor, and sqrt(4e12) = 2_000_000.
    register_stake(Some(4_000_000_000_000));
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("submit ok");

    gov.vote(id, accounts.bob, true).expect("vote ok");

    let proposal = gov.get_proposal(id).unwrap();
    // With the 1.0x time-staked multiplier, weight == sqrt(stake).
    assert_eq!(proposal.yes, 2_000_000);
    assert_eq!(proposal.no, 0);
}

#[ink::test]
fn vote_rejects_double_vote_for_same_pair() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    register_stake(Some(4_000_000_000_000));
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .expect("submit ok");

    gov.vote(id, accounts.bob, true).expect("vote ok");
    assert_eq!(gov.vote(id, accounts.bob, false), Err(Error::AlreadyVoted));
}

#[ink::test]
fn update_params_rejects_non_admin() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    set_caller(accounts.bob);
    let res = gov.update_params(GovernanceParams::default_params());
    assert!(matches!(res, Err(_)));
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
    // A non-zero circulating supply yields a non-zero quorum (20% of 100 = 20), so a
    // zero-vote proposal falls short and is rejected on finalize.
    let mut p = GovernanceParams::default_params();
    p.voting_period_ms = 1; // expire quickly
    gov.update_params(p).expect("update ok");
    gov.set_circulating_supply(100).expect("set supply ok");
    assert_eq!(gov.quorum(), 20);

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

    gov.set_circulating_supply(1_000_000).expect("set supply ok");
    assert_eq!(gov.circulating_supply(), 1_000_000);
    // 20% of 1_000_000.
    assert_eq!(gov.quorum(), 200_000);
}

#[ink::test]
fn set_circulating_supply_rejects_non_admin() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    set_caller(accounts.bob);
    assert_eq!(gov.set_circulating_supply(1_000_000), Err(Error::NotAdmin));
    assert_eq!(gov.circulating_supply(), 0);
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

    // sqrt(4e12) = 2_000_000 voting power from one vote. Set supply so quorum (20%) is below that.
    register_stake(Some(4_000_000_000_000));
    gov.set_circulating_supply(1_000_000).expect("set supply ok"); // quorum = 200_000
    assert!(gov.quorum() < 2_000_000);

    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding, accounts.bob)
        .unwrap();
    gov.vote(id, accounts.bob, true).expect("vote ok");

    ink::env::test::advance_block::<tusdt_env::CustomEnvironment>();
    ink::env::test::advance_block::<tusdt_env::CustomEnvironment>();

    gov.finalize(id).expect("finalize ok");
    assert_eq!(gov.get_proposal(id).unwrap().status, ProposalStatus::Passed);
}
