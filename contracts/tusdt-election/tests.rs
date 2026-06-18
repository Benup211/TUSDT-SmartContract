// Unit tests use ergonomic plain arithmetic on bounded constants; the strict numeric lints
// aren't useful here and would just clutter assertions.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::redundant_pattern_matching
)]

use super::election::*;
use ink::prelude::vec::Vec;
use ink::primitives::AccountId;
use tusdt_env::StakeInfo;

type Env = tusdt_env::CustomEnvironment;

const MS_PER_DAY: u64 = 86_400_000;
/// Mirrors the election contract's `MIN_CANDIDATE_STAKE` const (the "owner" bar).
const MIN_CANDIDATE_STAKE: u128 = 1_000_000_000_000;
/// A mocked stake comfortably above the registration floor.
const STAKE_ABOVE: u64 = 2_000_000_000_000;

/// Builds a Unix-epoch timestamp (ms) for the `day`-th of the test election month. Tests drive a
/// cycle in the first month on/after the genesis+term cadence anchor: with genesis 0 and a 730-day
/// term, that anchor is 1972-01-01 (a clean month start, day-of-month 1), so `day` maps to
/// `730 + (day - 1)` whole days past the epoch. This keeps the day-of-month gates (5th/10th/15th)
/// correct while sitting at/after the anchor that `schedule`/`test_schedule` enforce.
fn ts_for_day(day: u64) -> u64 {
    (730 + day - 1) * MS_PER_DAY
}

fn accounts() -> ink::env::test::DefaultAccounts<Env> {
    ink::env::test::default_accounts::<Env>()
}

fn set_caller(caller: AccountId) {
    let callee = ink::env::account_id::<Env>();
    ink::env::test::set_callee::<Env>(callee);
    ink::env::test::set_caller::<Env>(caller);
}

fn set_block_timestamp(ts: u64) {
    ink::env::test::set_block_timestamp::<Env>(ts);
}

/// Off-chain mock of the `get_stake_info_for_hotkey_coldkey_netuid` chain extension used by
/// `register_candidate`. `stake = Some(v)` resolves to a `StakeInfo` with that alpha stake;
/// `None` mimics a (hotkey, coldkey) pair with no stake record.
struct StakeExtension {
    stake: Option<u64>,
}

impl ink::env::test::ChainExtension for StakeExtension {
    fn ext_id(&self) -> u16 {
        0x1000
    }

    fn call(&mut self, _func_id: u16, _input: &[u8], output: &mut Vec<u8>) -> u32 {
        let a = accounts();
        let info = self.stake.map(|stake| StakeInfo {
            hotkey: a.alice,
            coldkey: a.alice,
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

/// Single-leaf electorate snapshot for voter (alice/alice, 4e12, 1.0x) on subnet 113.
fn alice_snapshot() -> ElectionSnapshot {
    let a = accounts();
    let root = leaf_hash(a.alice, a.alice, 4_000_000_000_000, 10_000);
    ElectionSnapshot {
        root,
        circulating_supply: 1_000_000,
        netuid: 113,
        snapshot_block: 0,
    }
}

/// Constructs an election whose genesis cadence anchor is `genesis` (the constructor reads it from
/// the block timestamp). Incumbent is frank (netuid 113); the incumbent runs the election machinery
/// (emergency, cancel, re-anchor). The governance address is a stand-in (unit tests don't make live
/// cross-contract calls; those are covered by e2e tests). A default above-floor stake is registered
/// so candidate registration succeeds unless overridden.
fn make_election_with_genesis(genesis: u64) -> TusdtElection {
    let a = accounts();
    set_caller(a.alice);
    set_block_timestamp(genesis);
    register_stake(Some(STAKE_ABOVE));
    TusdtElection::new(a.frank, a.frank, 113)
}

fn make_election() -> TusdtElection {
    let gov = make_election_with_genesis(0);
    // The first election is one full term after genesis; advance to that cadence anchor (day 1 of
    // the election month) so the cadence-gated `test_schedule` is allowed.
    set_block_timestamp(ts_for_day(1));
    gov
}

/// Drives a fresh election through schedule (injected snapshot) → register `candidate` for `netuid`
/// → advance to the 5th. Leaves the contract in `Registration`; the next `cast_approval` opens
/// voting lazily (there is no separate open step).
fn start_voting_with_candidate(gov: &mut TusdtElection, candidate: AccountId, netuid: u16) {
    gov.test_schedule(alice_snapshot()).expect("schedule");
    set_caller(candidate);
    gov.register_candidate(netuid, candidate).expect("register");
    set_block_timestamp(ts_for_day(5));
}

#[ink::test]
fn constructor_sets_defaults() {
    let a = accounts();
    let gov = make_election();
    assert_eq!(gov.phase(), Phase::Idle);
    assert_eq!(gov.incumbent(), a.frank);
    assert_eq!(gov.active_netuid(), 113);
    assert_eq!(gov.cycle_id(), 0);
    // The initial incumbent is treated as just elected, so the first election is one term out.
    assert_eq!(gov.next_election_ts(), 730 * MS_PER_DAY);
    assert_eq!(gov.min_candidate_stake(), MIN_CANDIDATE_STAKE);
    // The seated incumbent is counted as having served one term.
    assert_eq!(gov.terms_served(a.frank), 1);
    assert_eq!(gov.terms_served(a.alice), 0);
    assert!(gov.maintainer_elect().is_none());
    assert!(!gov.is_in_transition());
}

#[ink::test]
fn schedule_opens_registration() {
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    assert_eq!(gov.phase(), Phase::Registration);
    assert_eq!(gov.cycle_id(), 1);
}

#[ink::test]
fn schedule_rejects_before_anchor() {
    // Real `schedule_election` returns NotElectionTime before the governance read.
    let mut gov = make_election_with_genesis(ts_for_day(5));
    set_block_timestamp(0);
    assert_eq!(gov.schedule_election(), Err(Error::NotElectionTime));
    assert_eq!(gov.phase(), Phase::Idle);
}

#[ink::test]
fn schedule_rejects_when_not_idle() {
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    // Real `schedule_election` returns WrongPhase before the governance read.
    assert_eq!(gov.schedule_election(), Err(Error::WrongPhase));
}

#[ink::test]
fn emergency_bypasses_cadence_anchor() {
    let a = accounts();
    let mut gov = make_election_with_genesis(ts_for_day(20)); // anchor far in the future
    set_block_timestamp(0);
    // Without emergency the cadence blocks scheduling (before the governance read).
    assert_eq!(gov.schedule_election(), Err(Error::NotElectionTime));
    // The incumbent flags an emergency; scheduling is then allowed immediately.
    set_caller(a.frank);
    gov.trigger_emergency_election().expect("emergency");
    gov.test_schedule(alice_snapshot()).expect("emergency schedule");
    assert_eq!(gov.phase(), Phase::Registration);
}

#[ink::test]
fn trigger_emergency_requires_incumbent() {
    let a = accounts();
    let mut gov = make_election();
    set_caller(a.alice); // not the incumbent
    assert_eq!(gov.trigger_emergency_election(), Err(Error::NotIncumbent));
}

#[ink::test]
fn register_happy_path() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    set_caller(a.django);
    gov.register_candidate(200, a.django).expect("register");
    assert_eq!(gov.candidate_list(), ink::prelude::vec![a.django]);
    let c = gov.get_candidate(a.django).expect("candidacy");
    assert_eq!(c.netuid, 200);
}

#[ink::test]
fn register_rejects_outside_registration() {
    let a = accounts();
    let mut gov = make_election();
    set_caller(a.django);
    assert_eq!(gov.register_candidate(200, a.django), Err(Error::WrongPhase));
}

#[ink::test]
fn register_rejects_zero_netuid() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    set_caller(a.django);
    assert_eq!(gov.register_candidate(0, a.django), Err(Error::InvalidNetuid));
}

#[ink::test]
fn register_rejects_already_registered() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    set_caller(a.django);
    gov.register_candidate(200, a.django).expect("register");
    assert_eq!(
        gov.register_candidate(200, a.django),
        Err(Error::AlreadyRegistered)
    );
}

#[ink::test]
fn register_rejects_at_term_limit() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    gov.test_set_terms_served(a.django, 2);
    set_caller(a.django);
    assert_eq!(
        gov.register_candidate(200, a.django),
        Err(Error::TermLimitReached)
    );
}

#[ink::test]
fn register_rejects_insufficient_stake() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    // Below the owner floor.
    register_stake(Some(1));
    set_caller(a.django);
    assert_eq!(
        gov.register_candidate(200, a.django),
        Err(Error::InsufficientStake)
    );
}

#[ink::test]
fn register_rejects_no_stake_record() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    register_stake(None);
    set_caller(a.django);
    assert_eq!(gov.register_candidate(200, a.django), Err(Error::NoStake));
}

#[ink::test]
fn first_vote_before_day_five_is_rejected() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    set_caller(a.django);
    gov.register_candidate(200, a.django).expect("register");

    // Before the 5th the first vote cannot open voting; the cycle stays in Registration.
    let balance = 4_000_000_000_000u128;
    set_block_timestamp(ts_for_day(4));
    set_caller(a.alice);
    assert_eq!(
        gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new()),
        Err(Error::NotElectionTime)
    );
    assert_eq!(gov.phase(), Phase::Registration);
}

#[ink::test]
fn first_vote_opens_voting_window() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    set_caller(a.django);
    gov.register_candidate(200, a.django).expect("register");

    // The first vote on the 5th moves Registration → Voting and anchors the 5-day window to it.
    let balance = 4_000_000_000_000u128;
    set_block_timestamp(ts_for_day(5));
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new())
        .expect("first vote opens voting");
    assert_eq!(gov.phase(), Phase::Voting);
    let (opens, ends) = gov.voting_window();
    assert_eq!(opens, ts_for_day(5));
    assert_eq!(ends, ts_for_day(5) + 5 * MS_PER_DAY);
    // The opening vote is itself counted.
    assert_eq!(gov.approval_weight(a.django), 2_000_000);
    assert_eq!(gov.total_participating_power(), 2_000_000);
}

#[ink::test]
fn cast_approval_rejects_after_window_closes() {
    let a = accounts();
    let mut gov = make_election();
    start_voting_with_candidate(&mut gov, a.django, 200);
    let balance = 4_000_000_000_000u128;
    // A first vote on the 5th opens the window (closing on the 10th).
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new())
        .expect("approve");
    // Once the window has closed, further votes are rejected before any other check.
    set_block_timestamp(ts_for_day(10));
    set_caller(a.frank);
    assert_eq!(
        gov.cast_approval(a.django, a.frank, balance, 10_000, Vec::new()),
        Err(Error::VotingClosed)
    );
}

#[ink::test]
fn cast_approval_happy_path() {
    let a = accounts();
    let mut gov = make_election();
    start_voting_with_candidate(&mut gov, a.django, 200);

    let balance = 4_000_000_000_000u128;
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new())
        .expect("approve");
    // sqrt(4e12) = 2_000_000 at the 1.0x multiplier.
    assert_eq!(gov.approval_weight(a.django), 2_000_000);
    assert_eq!(gov.total_participating_power(), 2_000_000);
    assert_eq!(gov.total_voted_balance(), balance);
    assert_eq!(gov.leading_candidate(), Some((a.django, 2_000_000)));
}

#[ink::test]
fn cast_approval_applies_multiplier() {
    let a = accounts();
    let mut gov = make_election();
    // Snapshot tuned to an 0.8x multiplier leaf.
    let balance = 4_000_000_000_000u128;
    let root = leaf_hash(a.alice, a.alice, balance, 8_000);
    gov.test_schedule(ElectionSnapshot {
        root,
        circulating_supply: 1_000_000,
        netuid: 113,
        snapshot_block: 0,
    })
    .expect("schedule");
    set_caller(a.django);
    gov.register_candidate(200, a.django).expect("register");
    set_block_timestamp(ts_for_day(5));

    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 8_000, Vec::new())
        .expect("approve");
    // sqrt(4e12) = 2_000_000; 0.8x -> 1_600_000.
    assert_eq!(gov.approval_weight(a.django), 1_600_000);
}

#[ink::test]
fn cast_approval_rejects_invalid_proof() {
    let a = accounts();
    let mut gov = make_election();
    start_voting_with_candidate(&mut gov, a.django, 200);
    // A balance that does not match the committed leaf cannot be proven.
    set_caller(a.alice);
    assert_eq!(
        gov.cast_approval(a.django, a.alice, 8_000_000_000_000, 10_000, Vec::new()),
        Err(Error::InvalidProof)
    );
}

#[ink::test]
fn cast_approval_rejects_unregistered_candidate() {
    let a = accounts();
    let mut gov = make_election();
    start_voting_with_candidate(&mut gov, a.django, 200);
    let balance = 4_000_000_000_000u128;
    set_caller(a.alice);
    // eve never registered.
    assert_eq!(
        gov.cast_approval(a.eve, a.alice, balance, 10_000, Vec::new()),
        Err(Error::CandidateNotFound)
    );
}

#[ink::test]
fn cast_approval_rejects_double() {
    let a = accounts();
    let mut gov = make_election();
    start_voting_with_candidate(&mut gov, a.django, 200);
    let balance = 4_000_000_000_000u128;
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new())
        .expect("approve");
    assert_eq!(
        gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new()),
        Err(Error::AlreadyApproved)
    );
}

#[ink::test]
fn cast_approval_rejects_outside_voting() {
    let a = accounts();
    let mut gov = make_election();
    set_caller(a.alice);
    assert_eq!(
        gov.cast_approval(a.django, a.alice, 4_000_000_000_000, 10_000, Vec::new()),
        Err(Error::WrongPhase)
    );
}

#[ink::test]
fn finalize_records_winner_over_50_percent() {
    let a = accounts();
    let mut gov = make_election();
    start_voting_with_candidate(&mut gov, a.django, 200);
    let balance = 4_000_000_000_000u128;
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new())
        .expect("approve");

    set_block_timestamp(ts_for_day(10)); // == voting_ends_at
    gov.finalize().expect("finalize");
    assert_eq!(gov.phase(), Phase::Elected);
    let elect = gov.maintainer_elect().expect("winner");
    assert_eq!(elect.who, a.django);
    assert_eq!(elect.netuid, 200);
    assert_eq!(elect.approval_weight, 2_000_000);
    // Cadence does not advance until the winner activates; it stays at the initial one-term anchor.
    assert_eq!(gov.next_election_ts(), 730 * MS_PER_DAY);
}

#[ink::test]
fn finalize_tie_at_50_50_keeps_incumbent() {
    let a = accounts();
    let mut gov = make_election();

    // Two-leaf snapshot: voter alice and voter frank, equal weight.
    let balance = 4_000_000_000_000u128;
    let leaf_alice = leaf_hash(a.alice, a.alice, balance, 10_000);
    let leaf_frank = leaf_hash(a.frank, a.frank, balance, 10_000);
    let root = hash_pair(leaf_alice, leaf_frank);
    gov.test_schedule(ElectionSnapshot {
        root,
        circulating_supply: 1_000_000,
        netuid: 113,
        snapshot_block: 0,
    })
    .expect("schedule");

    // Two candidates register (mock stake is above the floor for both).
    set_caller(a.django);
    gov.register_candidate(200, a.django).expect("register django");
    set_caller(a.eve);
    gov.register_candidate(300, a.eve).expect("register eve");

    set_block_timestamp(ts_for_day(5));

    // alice approves django, frank approves eve — a clean 50/50 split.
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, ink::prelude::vec![leaf_frank])
        .expect("alice approves django");
    set_caller(a.frank);
    gov.cast_approval(a.eve, a.frank, balance, 10_000, ink::prelude::vec![leaf_alice])
        .expect("frank approves eve");

    assert_eq!(gov.total_participating_power(), 4_000_000);

    set_block_timestamp(ts_for_day(10));
    gov.finalize().expect("finalize");
    // Neither candidate strictly exceeds 50%, so the incumbent stays and the cadence advances.
    assert_eq!(gov.phase(), Phase::Idle);
    assert!(gov.maintainer_elect().is_none());
    assert_eq!(gov.incumbent(), a.frank);
    // The completed (no-winner) cycle advances the cadence one term, to the second anchor.
    assert_eq!(gov.next_election_ts(), 2 * 730 * MS_PER_DAY);
}

#[ink::test]
fn cast_approval_rejects_second_candidate() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    set_caller(a.django);
    gov.register_candidate(200, a.django).expect("register django");
    set_caller(a.eve);
    gov.register_candidate(300, a.eve).expect("register eve");
    set_block_timestamp(ts_for_day(5));

    let balance = 4_000_000_000_000u128;
    // Single vote: a leaf votes once; a second vote for a different candidate is rejected and the
    // leaf's power counts once toward the total.
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new())
        .expect("approve django");
    assert_eq!(
        gov.cast_approval(a.eve, a.alice, balance, 10_000, Vec::new()),
        Err(Error::AlreadyApproved)
    );
    assert_eq!(gov.approval_weight(a.django), 2_000_000);
    assert_eq!(gov.approval_weight(a.eve), 0);
    assert_eq!(gov.total_participating_power(), 2_000_000);
}

#[ink::test]
fn finalize_rejects_before_voting_ends() {
    let a = accounts();
    let mut gov = make_election();
    start_voting_with_candidate(&mut gov, a.django, 200);
    // A first vote on the 5th opens the window (closing on the 10th).
    let balance = 4_000_000_000_000u128;
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new())
        .expect("approve");
    set_block_timestamp(ts_for_day(6)); // still within the window
    assert_eq!(gov.finalize(), Err(Error::VotingStillOpen));
}

#[ink::test]
fn finalize_rejects_wrong_phase() {
    let mut gov = make_election();
    assert_eq!(gov.finalize(), Err(Error::WrongPhase));
}

#[ink::test]
fn activate_rejects_before_activation_day() {
    let a = accounts();
    let mut gov = make_election();
    start_voting_with_candidate(&mut gov, a.django, 200);
    let balance = 4_000_000_000_000u128;
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new())
        .expect("approve");
    set_block_timestamp(ts_for_day(10));
    gov.finalize().expect("finalize");

    // Activation is permissionless, but the 10th is before the 15th activation day, so even a
    // non-winner caller is rejected on the day gate (which precedes the cross-contract install).
    set_caller(a.bob);
    assert_eq!(gov.activate(), Err(Error::BeforeActivationDay));
}

#[ink::test]
fn activate_rejects_wrong_phase() {
    let a = accounts();
    let mut gov = make_election();
    set_caller(a.django);
    assert_eq!(gov.activate(), Err(Error::WrongPhase));
}

#[ink::test]
fn end_transition_rejects_when_none() {
    let mut gov = make_election();
    assert_eq!(gov.end_transition(), Err(Error::NoTransition));
}

#[ink::test]
fn end_transition_rejects_while_active() {
    let mut gov = make_election();
    gov.test_set_transition(Transition {
        from_netuid: 113,
        to_netuid: 200,
        ends_at: ts_for_day(28),
    });
    set_block_timestamp(ts_for_day(10)); // before ends_at
    assert_eq!(gov.end_transition(), Err(Error::TransitionActive));
    assert!(gov.is_in_transition());
}

#[ink::test]
fn cancel_cycle_resets_to_idle() {
    let a = accounts();
    let mut gov = make_election();
    set_caller(a.frank);
    // Cannot cancel when already idle.
    assert_eq!(gov.cancel_cycle(), Err(Error::WrongPhase));

    gov.test_schedule(alice_snapshot()).expect("schedule");
    set_caller(a.django);
    gov.register_candidate(200, a.django).expect("register");
    set_caller(a.frank);
    gov.cancel_cycle().expect("cancel");
    assert_eq!(gov.phase(), Phase::Idle);
    assert!(gov.candidate_list().is_empty());
}

#[ink::test]
fn cancel_cycle_requires_incumbent() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    set_caller(a.alice);
    assert_eq!(gov.cancel_cycle(), Err(Error::NotIncumbent));
}

#[ink::test]
fn cancel_cycle_rejected_outside_registration() {
    let a = accounts();
    let mut gov = make_election();
    start_voting_with_candidate(&mut gov, a.django, 200);
    // A first vote moves the cycle into Voting; cancellation is no longer allowed (the cycle is
    // finalizable after its window, so the incumbent cannot abort a live vote).
    let balance = 4_000_000_000_000u128;
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new())
        .expect("approve");
    set_caller(a.frank); // incumbent
    assert_eq!(gov.cancel_cycle(), Err(Error::WrongPhase));
    assert_eq!(gov.phase(), Phase::Voting);
}

#[ink::test]
fn voting_window_is_fixed_to_the_calendar() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    set_caller(a.django);
    gov.register_candidate(200, a.django).expect("register");

    // A first vote on the 7th (not the 5th) still closes the window on the 10th — the window is
    // anchored to the calendar, not to when the first vote lands.
    let balance = 4_000_000_000_000u128;
    set_block_timestamp(ts_for_day(7));
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new())
        .expect("first vote opens voting");
    let (opens, ends) = gov.voting_window();
    assert_eq!(opens, ts_for_day(7));
    assert_eq!(ends, ts_for_day(10));
}

#[ink::test]
fn first_vote_on_close_day_is_rejected() {
    let a = accounts();
    let mut gov = make_election();
    gov.test_schedule(alice_snapshot()).expect("schedule");
    set_caller(a.django);
    gov.register_candidate(200, a.django).expect("register");

    // On the 10th the window has closed for the month; the first vote cannot open it.
    let balance = 4_000_000_000_000u128;
    set_block_timestamp(ts_for_day(10));
    set_caller(a.alice);
    assert_eq!(
        gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new()),
        Err(Error::NotElectionTime)
    );
    assert_eq!(gov.phase(), Phase::Registration);
}

#[ink::test]
fn finalize_no_winner_when_quorum_unmet() {
    let a = accounts();
    let mut gov = make_election();
    // Circulating supply large enough that the 20% quorum exceeds the lone voter's balance.
    let balance = 4_000_000_000_000u128;
    let root = leaf_hash(a.alice, a.alice, balance, 10_000);
    gov.test_schedule(ElectionSnapshot {
        root,
        circulating_supply: 100_000_000_000_000, // quorum = 20e12 > 4e12 voted
        netuid: 113,
        snapshot_block: 0,
    })
    .expect("schedule");
    set_caller(a.django);
    gov.register_candidate(200, a.django).expect("register");

    set_block_timestamp(ts_for_day(5));
    set_caller(a.alice);
    gov.cast_approval(a.django, a.alice, balance, 10_000, Vec::new())
        .expect("approve");
    // The lone voter is 100% of participating power (> 50%), but turnout misses the quorum.
    assert_eq!(gov.quorum(), 20_000_000_000_000);
    assert_eq!(gov.total_voted_balance(), balance);

    set_block_timestamp(ts_for_day(10));
    gov.finalize().expect("finalize");
    assert_eq!(gov.phase(), Phase::Idle);
    assert!(gov.maintainer_elect().is_none());
    assert_eq!(gov.incumbent(), a.frank);
    // No winner: the cadence advances one term (to the second anchor) and the incumbent stays.
    assert_eq!(gov.next_election_ts(), 2 * 730 * MS_PER_DAY);
}

#[ink::test]
fn day_of_month_matches_known_dates() {
    assert_eq!(day_of_month(0), 1);
    assert_eq!(day_of_month(1_582_934_400_000), 29); // 2020-02-29 leap day
    assert_eq!(day_of_month(1_735_603_200_000), 31); // 2024-12-31
    assert_eq!(day_of_month(ts_for_day(5)), 5);
    assert_eq!(day_of_month(ts_for_day(15)), 15);
}

#[ink::test]
fn integer_sqrt_floors_correctly() {
    assert_eq!(integer_sqrt(0), 0);
    assert_eq!(integer_sqrt(8), 2);
    assert_eq!(integer_sqrt(9), 3);
    assert_eq!(integer_sqrt(4_000_000_000_000), 2_000_000);
}
