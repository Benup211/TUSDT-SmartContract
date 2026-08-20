#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::oracle::*;
use tusdt_primitives::Ratio;
use tusdt_test_support::{register_extension, set_caller, MockExtension};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn set_time(timestamp: u64) {
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(timestamp);
}

fn account_from_seed(seed: u32) -> ink::primitives::AccountId {
    let mut bytes = [0_u8; 32];
    bytes[..4].copy_from_slice(&seed.to_le_bytes());
    ink::primitives::AccountId::from(bytes)
}

fn metadata_for(reporter: ink::primitives::AccountId) -> PriceSubmissionMetadata {
    PriceSubmissionMetadata { hot_key: reporter, provider: None }
}

/// Helper: submit a price with default metadata (hot_key = caller, no provider).
fn submit_price(oracle: &mut TusdtOracle, reporter: ink::primitives::AccountId, price: u128) {
    set_caller(reporter);
    assert_eq!(oracle.submit_price(Ratio::from_integer(price), metadata_for(reporter)), Ok(()));
}

/// Helper: submit a price with custom metadata.
fn submit_price_with_metadata(
    oracle: &mut TusdtOracle,
    reporter: ink::primitives::AccountId,
    price: u128,
    metadata: PriceSubmissionMetadata,
) {
    set_caller(reporter);
    assert_eq!(oracle.submit_price(Ratio::from_integer(price), metadata), Ok(()));
}

// ---------------------------------------------------------------------------
// Chain-extension mock (shared `tusdt_test_support::MockExtension`, oracle knobs)
// ---------------------------------------------------------------------------

/// Default stake returned by the mock, above the 1e12 minimum threshold.
const MOCK_STAKE_ABOVE_THRESHOLD: u64 = 2_000_000_000_000;

/// Register a mock that reports a registered neuron with stake above the minimum.
fn register_stake(registered: bool) {
    register_extension(
        MockExtension::subnet_stake(Some(MOCK_STAKE_ABOVE_THRESHOLD))
            .with_is_registered(registered),
    );
}

/// Register a mock that returns a registered neuron but with stake below the minimum.
fn register_low_stake() {
    register_extension(MockExtension::subnet_stake(Some(1)));
}

/// Register a mock that returns no stake record at all.
fn register_no_stake() {
    register_extension(MockExtension::subnet_stake(None).with_is_registered(false));
}

/// Register a mock that simulates a chain-extension error (status code 1).
fn register_failing_extension() {
    register_extension(
        MockExtension::subnet_stake(None).with_is_registered(false).with_should_fail(true),
    );
}

// ========================================================================
// Existing tests — updated for subnet-based authorization
// ========================================================================

#[ink::test]
fn subnet_registration_is_required() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    register_no_stake();
    set_caller(accounts.bob);
    assert_eq!(
        oracle.submit_price(Ratio::from_integer(10), metadata_for(accounts.bob)),
        Err(Error::NotRegisteredInSubnet)
    );
}

#[ink::test]
fn zero_price_is_rejected() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    register_stake(true);
    set_caller(accounts.bob);
    assert_eq!(
        oracle.submit_price(Ratio::from_integer(0), metadata_for(accounts.bob)),
        Err(Error::InvalidPrice)
    );
}

#[ink::test]
fn reporter_resubmission_replaces_previous_value() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    register_stake(true);
    submit_price(&mut oracle, accounts.bob, 10);
    register_stake(true);
    submit_price(&mut oracle, accounts.bob, 12);
    register_stake(true);
    submit_price(&mut oracle, accounts.charlie, 13);
    register_stake(true);
    submit_price(&mut oracle, accounts.django, 14);

    assert_eq!(
        oracle.get_current_round_summary(),
        RoundSummary {
            round_id: 0,
            reporter_count: 3,
            median_price: Some(Ratio::from_integer(13)),
        }
    );
    assert_eq!(
        oracle.get_round_submissions(0),
        vec![
            PriceSubmission {
                reporter: accounts.bob,
                price: Ratio::from_integer(12),
                metadata: metadata_for(accounts.bob),
            },
            PriceSubmission {
                reporter: accounts.charlie,
                price: Ratio::from_integer(13),
                metadata: metadata_for(accounts.charlie),
            },
            PriceSubmission {
                reporter: accounts.django,
                price: Ratio::from_integer(14),
                metadata: metadata_for(accounts.django),
            },
        ]
    );
}

#[ink::test]
fn commit_is_blocked_below_quorum() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.django)), Ok(()));

    register_stake(true);
    submit_price(&mut oracle, accounts.bob, 10);
    register_stake(true);
    submit_price(&mut oracle, accounts.charlie, 20);

    set_caller(accounts.django);
    assert_eq!(oracle.commit_round(None), Err(Error::NotEnoughSubmissions));
}

#[ink::test]
fn override_allows_commit_without_submissions() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.bob)), Ok(()));

    set_time(55);
    set_caller(accounts.bob);
    let committed =
        oracle.commit_round(Some(Ratio::from_integer(42))).expect("override commit should succeed");

    assert_eq!(
        committed,
        PriceData {
            round_id: 0,
            price: Ratio::from_integer(42),
            median_price: Ratio::from_integer(42),
            reporter_count: 0,
            committed_at: 55,
            was_overridden: true,
        }
    );
    assert_eq!(oracle.get_latest_price(), Some(committed));
    assert_eq!(oracle.current_round_id(), 1);
}

#[ink::test]
fn override_bypasses_quorum_and_keeps_available_median() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.charlie)), Ok(()));

    register_stake(true);
    submit_price(&mut oracle, accounts.bob, 10);

    set_time(88);
    set_caller(accounts.charlie);
    let committed =
        oracle.commit_round(Some(Ratio::from_integer(25))).expect("override commit should succeed");

    assert_eq!(
        committed,
        PriceData {
            round_id: 0,
            price: Ratio::from_integer(25),
            median_price: Ratio::from_integer(10),
            reporter_count: 1,
            committed_at: 88,
            was_overridden: true,
        }
    );
}

#[ink::test]
fn median_is_used_for_three_submissions() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.eve)), Ok(()));

    register_stake(true);
    submit_price(&mut oracle, accounts.bob, 30);
    register_stake(true);
    submit_price(&mut oracle, accounts.charlie, 10);
    register_stake(true);
    submit_price(&mut oracle, accounts.django, 20);

    set_time(77);
    set_caller(accounts.eve);
    let committed = oracle.commit_round(None).expect("commit should succeed");
    assert_eq!(
        committed,
        PriceData {
            round_id: 0,
            price: Ratio::from_integer(20),
            median_price: Ratio::from_integer(20),
            reporter_count: 3,
            committed_at: 77,
            was_overridden: false,
        }
    );
}

#[ink::test]
fn median_is_used_for_five_submissions() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    register_stake(true);
    for reporter in [accounts.bob, accounts.charlie, accounts.django, accounts.eve, accounts.frank]
    {
        submit_price(
            &mut oracle,
            reporter,
            match reporter {
                r if r == accounts.bob => 50,
                r if r == accounts.charlie => 10,
                r if r == accounts.django => 30,
                r if r == accounts.eve => 20,
                r if r == accounts.frank => 40,
                _ => 0,
            },
        );
    }

    assert_eq!(
        oracle.get_current_round_summary(),
        RoundSummary {
            round_id: 0,
            reporter_count: 5,
            median_price: Some(Ratio::from_integer(30)),
        }
    );
}

#[ink::test]
fn median_is_averaged_for_four_submissions() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.frank)), Ok(()));

    register_stake(true);
    submit_price(&mut oracle, accounts.bob, 40);
    register_stake(true);
    submit_price(&mut oracle, accounts.charlie, 10);
    register_stake(true);
    submit_price(&mut oracle, accounts.django, 30);
    register_stake(true);
    submit_price(&mut oracle, accounts.eve, 20);

    assert_eq!(
        oracle.get_current_round_summary(),
        RoundSummary {
            round_id: 0,
            reporter_count: 4,
            median_price: Some(Ratio::from_integer(25)),
        }
    );

    set_time(123);
    set_caller(accounts.frank);
    let committed = oracle.commit_round(None).expect("commit should succeed");
    assert_eq!(
        committed,
        PriceData {
            round_id: 0,
            price: Ratio::from_integer(25),
            median_price: Ratio::from_integer(25),
            reporter_count: 4,
            committed_at: 123,
            was_overridden: false,
        }
    );
}

#[ink::test]
fn manual_override_is_stored_while_preserving_median_metadata() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.eve)), Ok(()));

    register_stake(true);
    submit_price(&mut oracle, accounts.bob, 10);
    register_stake(true);
    submit_price(&mut oracle, accounts.charlie, 20);
    register_stake(true);
    submit_price(&mut oracle, accounts.django, 30);

    set_time(99);
    set_caller(accounts.eve);
    let committed =
        oracle.commit_round(Some(Ratio::from_integer(99))).expect("override commit should succeed");

    assert_eq!(
        committed,
        PriceData {
            round_id: 0,
            price: Ratio::from_integer(99),
            median_price: Ratio::from_integer(20),
            reporter_count: 3,
            committed_at: 99,
            was_overridden: true,
        }
    );
    assert_eq!(oracle.get_latest_price(), Some(committed));
}

#[ink::test]
fn commit_advances_the_round() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.eve)), Ok(()));

    register_stake(true);
    submit_price(&mut oracle, accounts.bob, 10);
    register_stake(true);
    submit_price(&mut oracle, accounts.charlie, 20);
    register_stake(true);
    submit_price(&mut oracle, accounts.django, 30);

    set_caller(accounts.eve);
    oracle.commit_round(None).expect("commit should succeed");

    assert_eq!(oracle.current_round_id(), 1);
    assert_eq!(
        oracle.get_current_round_summary(),
        RoundSummary { round_id: 1, reporter_count: 0, median_price: None }
    );
}

#[ink::test]
fn governance_sets_validator() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.bob);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.bob, 113);

    assert_eq!(oracle.set_validator(Some(accounts.charlie)), Ok(()));
    assert_eq!(oracle.validator(), Some(accounts.charlie));

    set_caller(accounts.eve);
    assert_eq!(oracle.set_validator(Some(accounts.eve)), Err(Error::NotGovernance));
}

#[ink::test]
fn controller_updates_oracle_governance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.bob, 113);

    set_caller(accounts.bob);
    assert_eq!(oracle.update_governance(accounts.charlie), Err(Error::NotController));

    set_caller(accounts.alice);
    assert_eq!(oracle.update_governance(accounts.charlie), Ok(()));
    assert_eq!(oracle.governance(), accounts.charlie);

    // Verify the old governance can no longer act.
    set_caller(accounts.bob);
    assert_eq!(
        oracle.set_max_price_deviation(Ratio::from_basis_points(1_000)),
        Err(Error::NotGovernance)
    );

    set_caller(accounts.charlie);
    assert_eq!(oracle.set_max_price_deviation(Ratio::from_basis_points(1_000)), Ok(()));
}

#[ink::test]
fn committed_round_history_is_queryable() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    set_time(10);
    let round_0 = oracle
        .commit_round_governance(Ratio::from_integer(11))
        .expect("first governance commit should succeed");

    set_time(20);
    let round_1 = oracle
        .commit_round_governance(Ratio::from_integer(22))
        .expect("second governance commit should succeed");

    assert_eq!(oracle.get_round_price(0), Some(round_0));
    assert_eq!(oracle.get_round_price(1), Some(round_1));
    assert_eq!(oracle.get_price_history_count(), 2);
    assert_eq!(oracle.get_price_history(0), vec![round_1, round_0]);
    assert_eq!(oracle.get_price_history(1), Vec::<PriceData>::new());
}

#[ink::test]
fn committed_round_history_supports_pagination() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    for round_id in 0..12_u32 {
        set_time(round_id as u64);
        oracle
            .commit_round_governance(Ratio::from_integer(round_id as u128 + 1))
            .expect("governance commit should succeed");
    }

    assert_eq!(oracle.get_price_history_count(), 12);

    let page_0 = oracle.get_price_history(0);
    assert_eq!(page_0.len(), 10);
    assert_eq!(page_0[0].round_id, 11);
    assert_eq!(page_0[9].round_id, 2);

    let page_1 = oracle.get_price_history(1);
    assert_eq!(page_1.len(), 2);
    assert_eq!(page_1[0].round_id, 1);
    assert_eq!(page_1[1].round_id, 0);

    assert!(oracle.get_price_history(2).is_empty());
}

#[ink::test]
fn round_submissions_include_metadata() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    register_stake(true);
    submit_price_with_metadata(
        &mut oracle,
        accounts.bob,
        12,
        PriceSubmissionMetadata { hot_key: accounts.eve, provider: None },
    );
    register_stake(true);
    submit_price(&mut oracle, accounts.charlie, 15);

    assert_eq!(
        oracle.get_round_submissions(0),
        vec![
            PriceSubmission {
                reporter: accounts.bob,
                price: Ratio::from_integer(12),
                metadata: PriceSubmissionMetadata { hot_key: accounts.eve, provider: None },
            },
            PriceSubmission {
                reporter: accounts.charlie,
                price: Ratio::from_integer(15),
                metadata: metadata_for(accounts.charlie),
            },
        ]
    );
}

#[ink::test]
fn round_submission_count_is_bounded() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    let max_submissions = oracle.max_round_submissions();
    for seed in 0..max_submissions {
        let reporter = account_from_seed(seed + 1);
        register_stake(true);
        submit_price(&mut oracle, reporter, seed as u128 + 1);
    }

    let overflow_reporter = account_from_seed(max_submissions + 1);
    register_stake(true);
    set_caller(overflow_reporter);
    assert_eq!(
        oracle.submit_price(Ratio::from_integer(999), metadata_for(overflow_reporter)),
        Err(Error::MaxSubmissionsReached)
    );

    assert_eq!(oracle.get_current_round_summary().reporter_count, max_submissions);
    assert_eq!(oracle.get_round_submissions(0).len(), max_submissions as usize);
}

#[ink::test]
fn first_commit_skips_deviation_check() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.bob)), Ok(()));

    set_caller(accounts.bob);
    assert!(oracle.commit_round(Some(Ratio::from_integer(1_000))).is_ok());
}

#[ink::test]
fn validator_commit_within_deviation_succeeds() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.bob)), Ok(()));

    set_caller(accounts.bob);
    oracle.commit_round(Some(Ratio::from_integer(100))).expect("first commit should succeed");
    // 5% default deviation: 104 is within 95..=105.
    assert!(oracle.commit_round(Some(Ratio::from_integer(104))).is_ok());
}

#[ink::test]
fn validator_commit_outside_deviation_is_rejected() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.bob)), Ok(()));

    set_caller(accounts.bob);
    oracle.commit_round(Some(Ratio::from_integer(100))).expect("first commit should succeed");
    assert_eq!(
        oracle.commit_round(Some(Ratio::from_integer(130))),
        Err(Error::PriceDeviationExceeded)
    );
}

#[ink::test]
fn median_commit_outside_deviation_is_rejected() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.eve)), Ok(()));

    set_caller(accounts.eve);
    oracle.commit_round(Some(Ratio::from_integer(100))).expect("first commit should succeed");

    register_stake(true);
    submit_price(&mut oracle, accounts.bob, 200);
    register_stake(true);
    submit_price(&mut oracle, accounts.charlie, 210);
    register_stake(true);
    submit_price(&mut oracle, accounts.django, 220);

    set_caller(accounts.eve);
    assert_eq!(oracle.commit_round(None), Err(Error::PriceDeviationExceeded));
}

#[ink::test]
fn governance_commit_bypasses_deviation_and_quorum() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.bob)), Ok(()));

    set_caller(accounts.bob);
    oracle.commit_round(Some(Ratio::from_integer(100))).expect("first commit should succeed");
    assert_eq!(
        oracle.commit_round(Some(Ratio::from_integer(500))),
        Err(Error::PriceDeviationExceeded)
    );

    set_caller(accounts.alice);
    set_time(42);
    let committed = oracle
        .commit_round_governance(Ratio::from_integer(500))
        .expect("governance commit should bypass deviation");
    assert_eq!(committed.price, Ratio::from_integer(500));
    assert!(committed.was_overridden);
    assert_eq!(committed.committed_at, 42);
    assert_eq!(oracle.get_latest_price(), Some(committed));
}

#[ink::test]
fn governance_commit_rejects_zero_price() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    assert_eq!(oracle.commit_round_governance(Ratio::from_integer(0)), Err(Error::InvalidPrice));
}

#[ink::test]
fn governance_commit_requires_governance_caller() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    set_caller(accounts.bob);
    assert_eq!(oracle.commit_round_governance(Ratio::from_integer(10)), Err(Error::NotGovernance));
}

#[ink::test]
fn governance_can_widen_deviation_threshold() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);
    assert_eq!(oracle.set_validator(Some(accounts.bob)), Ok(()));

    set_caller(accounts.bob);
    oracle.commit_round(Some(Ratio::from_integer(100))).expect("first commit should succeed");
    assert_eq!(
        oracle.commit_round(Some(Ratio::from_integer(130))),
        Err(Error::PriceDeviationExceeded)
    );

    set_caller(accounts.alice);
    // Allow up to 50% deviation.
    assert_eq!(oracle.set_max_price_deviation(Ratio::from_basis_points(5_000)), Ok(()));
    assert_eq!(oracle.max_price_deviation(), Ratio::from_basis_points(5_000));

    set_caller(accounts.bob);
    assert!(oracle.commit_round(Some(Ratio::from_integer(130))).is_ok());
}

#[ink::test]
fn set_max_price_deviation_requires_governance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    set_caller(accounts.bob);
    assert_eq!(
        oracle.set_max_price_deviation(Ratio::from_basis_points(1_000)),
        Err(Error::NotGovernance)
    );
}

// ========================================================================
// New tests — subnet-based authorization
// ========================================================================

#[ink::test]
fn invalid_hotkey_rejected() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    register_stake(true);
    let zero_hotkey = ink::primitives::AccountId::from([0u8; 32]);
    set_caller(accounts.bob);
    assert_eq!(
        oracle.submit_price(
            Ratio::from_integer(10),
            PriceSubmissionMetadata { hot_key: zero_hotkey, provider: None }
        ),
        Err(Error::InvalidHotkey)
    );
}

#[ink::test]
fn not_registered_in_subnet_rejected() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    // Mock reports stake exists but is_registered = false.
    register_stake(false);
    set_caller(accounts.bob);
    assert_eq!(
        oracle.submit_price(Ratio::from_integer(10), metadata_for(accounts.bob)),
        Err(Error::NotRegisteredInSubnet)
    );
}

#[ink::test]
fn no_stake_record_rejected() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    // Mock returns None (no stake record at all).
    register_no_stake();
    set_caller(accounts.bob);
    assert_eq!(
        oracle.submit_price(Ratio::from_integer(10), metadata_for(accounts.bob)),
        Err(Error::NotRegisteredInSubnet)
    );
}

#[ink::test]
fn chain_extension_failure_rejected() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    register_failing_extension();
    set_caller(accounts.bob);
    assert_eq!(
        oracle.submit_price(Ratio::from_integer(10), metadata_for(accounts.bob)),
        Err(Error::ChainExtensionFailed)
    );
}

#[ink::test]
fn provider_field_persisted() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    register_stake(true);
    let metadata =
        PriceSubmissionMetadata { hot_key: accounts.bob, provider: Some(b"coingecko".to_vec()) };
    submit_price_with_metadata(&mut oracle, accounts.bob, 100, metadata);

    let submissions = oracle.get_round_submissions(0);
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].reporter, accounts.bob);
    assert_eq!(submissions[0].price, Ratio::from_integer(100));
    assert_eq!(submissions[0].metadata.hot_key, accounts.bob);
    assert_eq!(submissions[0].metadata.provider, Some(b"coingecko".to_vec()));
}

#[ink::test]
fn set_netuid_and_get_netuid() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 199);

    assert_eq!(oracle.get_netuid(), 199);

    // Governance can change it.
    assert_eq!(oracle.set_netuid(42), Ok(()));
    assert_eq!(oracle.get_netuid(), 42);
}

#[ink::test]
fn set_netuid_requires_governance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.bob, 113);

    // alice is the controller, not governance; bob is governance.
    set_caller(accounts.alice);
    assert_eq!(oracle.set_netuid(42), Err(Error::NotGovernance));

    // bob (governance) can set it.
    set_caller(accounts.bob);
    assert_eq!(oracle.set_netuid(42), Ok(()));
}

#[ink::test]
fn insufficient_stake_rejected() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    register_low_stake();
    set_caller(accounts.bob);
    assert_eq!(
        oracle.submit_price(Ratio::from_integer(10), metadata_for(accounts.bob)),
        Err(Error::InsufficientStake)
    );
}

#[ink::test]
fn set_min_submitter_stake_and_get() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.alice, 113);

    assert_eq!(oracle.min_submitter_stake(), 10_000_000_000);
    assert_eq!(oracle.set_min_submitter_stake(500_000_000_000), Ok(()));
    assert_eq!(oracle.min_submitter_stake(), 500_000_000_000);
}

#[ink::test]
fn set_min_submitter_stake_requires_governance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_caller(accounts.alice);
    let mut oracle = TusdtOracle::new(accounts.alice, accounts.bob, 113);

    // alice is the controller, not governance; bob is governance.
    set_caller(accounts.alice);
    assert_eq!(oracle.set_min_submitter_stake(100), Err(Error::NotGovernance));

    set_caller(accounts.bob);
    assert_eq!(oracle.set_min_submitter_stake(100), Ok(()));
    assert_eq!(oracle.min_submitter_stake(), 100);
}
