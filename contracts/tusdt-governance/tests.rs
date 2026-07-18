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
use tusdt_primitives::Ratio;
use tusdt_treasury::{Fund, TokenKind};
use tusdt_vault_alpha::VaultGlobalParamsConfig;

const MS_PER_DAY: u64 = 86_400_000;

/// Builds a Unix-epoch timestamp (ms) for the `day`-th of the month. Epoch day 0 is
/// 1970-01-01 (the 1st), so `day` maps to `(day - 1)` whole days past the epoch.
fn ts_for_day(day: u64) -> u64 {
    (day - 1) * MS_PER_DAY
}

/// A timestamp comfortably inside the 20..=27 submission window (the 23rd).
const IN_WINDOW_TS: u64 = 22 * MS_PER_DAY;

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
    // Default the clock inside the submission window; individual tests can override.
    set_block_timestamp(IN_WINDOW_TS);
    // Maintainer is alice (the caller). The treasury/vault/auction/oracle/election addresses are
    // stand-in accounts: unit tests exercise authorization, not live cross-contract calls (those
    // require a deployed callee), so any valid AccountId works here. frank stands in for the
    // election contract (it's also the non-council account used by negative tests).
    let mut gov = TusdtGovernance::from_addresses(
        accounts.django,
        accounts.eve,
        accounts.frank,
        accounts.charlie,
        accounts.alice,
        accounts.frank,
    );
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
    // netuid is a top-level field (bound to the maintainer), not a tunable param.
    assert_eq!(gov.netuid(), 113);
    let p = gov.params();
    assert_eq!(p.voting_period_ms, 7 * 24 * 60 * 60 * 1_000);
    assert_eq!(p.approval_bps, 5_001);
    assert_eq!(p.quorum_bps, 2_000);
    assert_eq!(p.min_proposer_stake, 1_000_000_000_000);
    assert_eq!(p.submission_open_day, 20);
    assert_eq!(p.submission_close_day, 27);
    // No snapshot yet → no epoch, no snapshot record, quorum threshold of zero.
    assert_eq!(gov.current_epoch(), 0);
    assert!(gov.get_snapshot(1).is_none());
    assert_eq!(gov.quorum(1), 0);
}

#[ink::test]
fn submit_proposal_rejects_without_snapshot() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance_no_snapshot();
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding);
    assert_eq!(res, Err(Error::NoSnapshot));
}

#[ink::test]
fn submit_proposal_rejects_empty_cid() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let res = gov.submit_proposal(String::new(), ProposalKind::NonFunding);
    assert!(matches!(res, Err(_)));
}

#[ink::test]
fn submit_proposal_rejects_oversized_cid() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let long_cid = String::from_utf8(vec![b'a'; 97]).unwrap();
    let res = gov.submit_proposal(long_cid, ProposalKind::NonFunding);
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
        )
        .expect("submit ok");
    assert_eq!(id2, 2);
    assert_eq!(gov.proposal_count(), 2);
}

#[ink::test]
fn submit_proposal_rejects_non_council() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // frank is intentionally not in the council.
    set_caller(accounts.frank);
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding);
    assert_eq!(res, Err(Error::NotCouncil));
    assert_eq!(gov.proposal_count(), 0);
}

#[ink::test]
fn submit_proposal_rejects_before_window_opens() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // The 19th is before the default open day (20th).
    set_block_timestamp(ts_for_day(19));
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding);
    assert_eq!(res, Err(Error::OutsideSubmissionWindow));
    assert_eq!(gov.proposal_count(), 0);
}

#[ink::test]
fn submit_proposal_rejects_after_window_closes() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // The 28th is after the default close day (27th).
    set_block_timestamp(ts_for_day(28));
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding);
    assert_eq!(res, Err(Error::OutsideSubmissionWindow));
    assert_eq!(gov.proposal_count(), 0);
}

#[ink::test]
fn submit_proposal_allows_on_window_boundaries() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();

    // Both the open (20th) and close (27th) days are inclusive.
    set_block_timestamp(ts_for_day(20));
    gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
        .expect("20th accepted");

    set_block_timestamp(ts_for_day(27));
    gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
        .expect("27th accepted");

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

    // Narrow the window to the 10th–12th; the 15th now falls outside it.
    let mut p = GovernanceParams::default_params();
    p.submission_open_day = 10;
    p.submission_close_day = 12;
    gov.update_params(p).expect("update ok");

    set_block_timestamp(ts_for_day(15));
    let res = gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding);
    assert_eq!(res, Err(Error::OutsideSubmissionWindow));

    set_block_timestamp(ts_for_day(11));
    gov.submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
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
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
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
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
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
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
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
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
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
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
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
    p.quorum_bps = 3_000;
    p.approval_bps = 6_667;
    p.voting_period_ms = 60 * 1_000;
    gov.update_params(p).expect("update ok");
    assert_eq!(gov.params().quorum_bps, 3_000);
    assert_eq!(gov.params().approval_bps, 6_667);
    assert_eq!(gov.params().voting_period_ms, 60 * 1_000);
}

#[ink::test]
fn finalize_before_voting_ends_fails() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
        .unwrap();
    assert!(gov.finalize(id).is_err());
}

#[ink::test]
fn execute_on_non_passed_fails() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    let id = gov
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
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
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
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
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
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
        .submit_proposal(String::from("bafy"), ProposalKind::NonFunding)
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
fn election_installs_maintainer() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    // The election contract is fixed at construction (frank stands in for it).
    assert_eq!(gov.election(), accounts.frank);

    // Only the election may install the maintainer; the maintainer cannot self-transfer.
    set_caller(accounts.alice);
    assert_eq!(gov.elect_maintainer(accounts.bob), Err(Error::NotElection));

    // The election installs bob, and can apply a netuid switch.
    set_caller(accounts.frank);
    gov.elect_maintainer(accounts.bob).expect("elect ok");
    assert_eq!(gov.maintainer(), accounts.bob);
    gov.election_set_netuid(99).expect("netuid ok");
    assert_eq!(gov.netuid(), 99);

    // The new maintainer governs; the old one no longer does.
    set_caller(accounts.bob);
    gov.update_params(GovernanceParams::default_params())
        .expect("update ok");
    set_caller(accounts.alice);
    assert_eq!(
        gov.update_params(GovernanceParams::default_params()),
        Err(Error::NotMaintainer)
    );
}

#[ink::test]
fn election_entries_reject_non_election_callers() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    // Neither the maintainer (alice) nor a council member can drive the election-gated entries.
    set_caller(accounts.alice);
    assert_eq!(gov.elect_maintainer(accounts.bob), Err(Error::NotElection));
    assert_eq!(gov.election_set_netuid(7), Err(Error::NotElection));
    set_caller(accounts.bob);
    assert_eq!(gov.elect_maintainer(accounts.bob), Err(Error::NotElection));
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

// ---------------------------------------------------------------------------------------------
// Cross-contract forwarder authorization. These assert the role gate rejects the wrong caller
// *before* any cross-contract call is attempted (a live call would need a deployed callee, which
// off-chain unit tests don't provide). The happy paths are covered by e2e tests, not here.
// ---------------------------------------------------------------------------------------------

#[ink::test]
fn vault_maintainer_forwarders_reject_non_maintainer() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    // bob is a council member but not the maintainer.
    set_caller(accounts.bob);
    assert_eq!(
        gov.vault_update_treasury(accounts.bob),
        Err(Error::NotMaintainer)
    );
    assert_eq!(
        gov.vault_update_platform(accounts.bob),
        Err(Error::NotMaintainer)
    );
    assert_eq!(gov.vault_unpause(), Err(Error::NotMaintainer));
    assert_eq!(
        gov.vault_cancel_contract_params_update(1),
        Err(Error::NotMaintainer)
    );
    assert_eq!(
        gov.vault_set_approved_netuid(1, true),
        Err(Error::NotMaintainer)
    );
    assert_eq!(
        gov.vault_set_global_params(VaultGlobalParamsConfig {
            transaction_fee: 30,
            auction_duration_ms: 3_600_000,
            max_oracle_age_ms: 1_800_000,
            vault_creation_fee: 0,
        }),
        Err(Error::NotMaintainer)
    );
    assert_eq!(
        gov.vault_cancel_global_params_update(),
        Err(Error::NotMaintainer)
    );
}

#[ink::test]
fn oracle_and_auction_maintainer_forwarders_reject_non_maintainer() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    set_caller(accounts.bob);
    assert_eq!(
        gov.oracle_set_validator(Some(accounts.bob)),
        Err(Error::NotMaintainer)
    );
    assert_eq!(
        gov.oracle_set_max_price_deviation(Ratio::from_basis_points(500)),
        Err(Error::NotMaintainer)
    );
    assert_eq!(
        gov.auction_set_admin(Some(accounts.bob)),
        Err(Error::NotMaintainer)
    );
    assert_eq!(
        gov.oracle_commit_round(Ratio::from_basis_points(10_000)),
        Err(Error::NotMaintainer)
    );
}

#[ink::test]
fn council_forwarders_reject_non_council() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut gov = make_governance();
    // frank is neither maintainer nor a council member.
    set_caller(accounts.frank);
    assert_eq!(gov.vault_pause(), Err(Error::NotCouncil));
}
