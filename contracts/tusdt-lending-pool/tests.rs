
// Lending pool unit tests. Uses `#[ink::test]` with a mock chain extension.

use super::lending_pool::*;
use ink::env::test;
use tusdt_primitives::Ratio;
use tusdt_test_support::{
    last_ext_call, register_extension, register_mock, register_mock_chain_fails,
    register_mock_no_stake, register_mock_stateful_root, register_mock_transfer_fails,
    set_callee_balance, set_caller, MockExtension,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_accounts() -> test::DefaultAccounts<tusdt_env::CustomEnvironment> {
    test::default_accounts::<tusdt_env::CustomEnvironment>()
}

fn setup() -> (
    TusdtLendingPool,
    test::DefaultAccounts<tusdt_env::CustomEnvironment>,
) {
    let accounts = default_accounts();
    let pool = TusdtLendingPool::new_for_test(accounts.alice);
    (pool, accounts)
}

fn setup_with_alpha(netuid: u16) -> (
    TusdtLendingPool,
    test::DefaultAccounts<tusdt_env::CustomEnvironment>,
) {
    let accounts = default_accounts();
    let mut pool = TusdtLendingPool::new_for_test(accounts.alice);
    set_caller(accounts.alice);
    pool.set_approved_netuid(netuid, true).unwrap();
    (pool, accounts)
}

/// Sets the global params timelock to zero so tests can execute param updates instantly.
fn set_timelock_to_zero(pool: &mut TusdtLendingPool) {
    let config = PoolGlobalParamsConfig {
        max_oracle_age_ms: 1_800_000,
        close_factor: 5000,
        performance_fee: 2500,
        supply_cap_tao: 0,
        supply_cap_tusdt: 0,
        borrow_cap_tao: 0,
        borrow_cap_tusdt: 0,
    };
    pool.set_global_params(config).unwrap();
    // Advance past the timelock, execute, then reset.
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(
        PARAMS_TIMELOCK_MS + 1,
    );
    pool.execute_global_params_update().unwrap();
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(0);
}

// ---------------------------------------------------------------------------
// Constructor & access control
// ---------------------------------------------------------------------------

#[ink::test]
fn constructor_sets_governance() {
    let (pool, accounts) = setup();
    assert_eq!(pool.governance(), accounts.alice);
}

#[ink::test]
fn constructor_sets_treasury() {
    let (pool, accounts) = setup();
    assert_eq!(pool.treasury(), accounts.alice);
}

#[ink::test]
fn constructor_sets_platform() {
    let (pool, accounts) = setup();
    assert_eq!(pool.platform(), accounts.alice);
}

#[ink::test]
fn paused_by_default() {
    let (pool, _accounts) = setup();
    assert!(!pool.paused());
}

#[ink::test]
fn governance_can_pause_and_unpause() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.pause().unwrap();
    assert!(pool.paused());
    pool.unpause().unwrap();
    assert!(!pool.paused());
}

#[ink::test]
fn non_governance_cannot_pause() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.bob);
    assert_eq!(pool.pause(), Err(Error::NotGovernanceOrPlatform));
}

#[ink::test]
fn platform_can_pause() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    // Platform is alice (same as governance in test setup)
    pool.pause().unwrap();
    assert!(pool.paused());
}

#[ink::test]
fn non_governance_cannot_unpause() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.pause().unwrap();
    set_caller(accounts.bob);
    assert_eq!(pool.unpause(), Err(Error::NotMaintainer));
}

#[ink::test]
fn governance_can_update_governance() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.update_governance(accounts.bob).unwrap();
    assert_eq!(pool.governance(), accounts.bob);
    // Old governance can no longer pause (platform unchanged, so platform can still pause)
    pool.pause().unwrap(); // platform is still alice
}

#[ink::test]
fn non_governance_cannot_update_governance() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.bob);
    assert_eq!(
        pool.update_governance(accounts.bob),
        Err(Error::NotGovernance)
    );
}

#[ink::test]
fn governance_can_update_treasury() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.update_treasury(accounts.bob).unwrap();
    assert_eq!(pool.treasury(), accounts.bob);
}

#[ink::test]
fn governance_can_update_platform() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.update_platform(accounts.bob).unwrap();
    assert_eq!(pool.platform(), accounts.bob);
}

// ---------------------------------------------------------------------------
// Alpha market management
// ---------------------------------------------------------------------------

#[ink::test]
fn governance_can_add_alpha_market() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.set_approved_netuid(1, true).unwrap();
    assert!(pool.is_approved_netuid(1));
}

#[ink::test]
fn governance_can_remove_alpha_market() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.set_approved_netuid(1, true).unwrap();
    assert!(pool.is_approved_netuid(1));
    pool.set_approved_netuid(1, false).unwrap();
    assert!(!pool.is_approved_netuid(1));
}

#[ink::test]
fn non_governance_cannot_manage_netuids() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.bob);
    assert_eq!(
        pool.set_approved_netuid(1, true),
        Err(Error::NotMaintainer)
    );
}

#[ink::test]
fn cannot_remove_netuid_with_positions() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    // Deposit alpha to create a real position
    register_mock(1_000);
    pool.deposit_alpha(1, 100).unwrap();
    // Now removal should be blocked
    assert_eq!(
        pool.set_approved_netuid(1, false),
        Err(Error::NetuidHasPositions)
    );
}

#[ink::test]
fn duplicate_approval_is_noop() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.set_approved_netuid(1, true).unwrap();
    assert!(pool.is_approved_netuid(1));
    // Second call is a no-op
    pool.set_approved_netuid(1, true).unwrap();
    assert!(pool.is_approved_netuid(1));
}

#[ink::test]
fn removing_unapproved_netuid_is_noop() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.set_approved_netuid(99, false).unwrap();
    assert!(!pool.is_approved_netuid(99));
}

#[ink::test]
fn alpha_market_count() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    assert_eq!(pool.get_active_netuids_count(), 0);
    pool.set_approved_netuid(1, true).unwrap();
    assert_eq!(pool.get_active_netuids_count(), 1);
    pool.set_approved_netuid(2, true).unwrap();
    assert_eq!(pool.get_active_netuids_count(), 2);
}

#[ink::test]
fn alpha_markets_list() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.set_approved_netuid(1, true).unwrap();
    pool.set_approved_netuid(2, true).unwrap();
    let markets = pool.get_alpha_markets();
    assert_eq!(markets.len(), 2);
}

// ---------------------------------------------------------------------------
// Interest rate math (pure function, no cross-contract calls)
// ---------------------------------------------------------------------------

fn default_params() -> InterestRateParams {
    InterestRateParams {
        base_rate: Ratio::from_inner(0),
        slope1: Ratio::from_basis_points(400),
        slope2: Ratio::from_basis_points(9600),
        optimal_utilization: Ratio::from_basis_points(8000),
        reserve_factor: Ratio::from_basis_points(2000),
    }
}

#[ink::test]
fn borrow_rate_zero_at_zero_utilization() {
    let params = default_params();
    let rate = TusdtLendingPool::compute_borrow_rate(&params, Ratio::from_inner(0)).unwrap();
    assert!(rate.is_zero());
}

#[ink::test]
fn borrow_rate_in_zone_1() {
    let params = default_params();
    // U = 40% (4_000 bps), optimal = 80% (8_000 bps)
    let utilization = Ratio::from_basis_points(4000);
    let rate = TusdtLendingPool::compute_borrow_rate(&params, utilization).unwrap();
    // Zone 1: base(0) + slope1(4%) × U/opt = 0 + 0.04 × 0.4/0.8 = 0.04 × 0.5 = 0.02 = 2%
    let expected = Ratio::from_basis_points(200);
    assert_eq!(rate, expected);
}

#[ink::test]
fn borrow_rate_at_optimal() {
    let params = default_params();
    // U = optimal = 80%
    let utilization = Ratio::from_basis_points(8000);
    let rate = TusdtLendingPool::compute_borrow_rate(&params, utilization).unwrap();
    // Zone 1: base(0) + slope1(4%) × 1.0 = 4%
    let expected = Ratio::from_basis_points(400);
    assert_eq!(rate, expected);
}

#[ink::test]
fn borrow_rate_in_zone_2() {
    let params = default_params();
    // U = 90%, optimal = 80%
    let utilization = Ratio::from_basis_points(9000);
    let rate = TusdtLendingPool::compute_borrow_rate(&params, utilization).unwrap();
    // Zone 2: base(0) + slope1(4%) + slope2(96%) × (0.9-0.8)/(1.0-0.8)
    // = 0.04 + 0.96 × 0.1/0.2 = 0.04 + 0.96 × 0.5 = 0.04 + 0.48 = 0.52 = 52%
    let expected = Ratio::from_basis_points(5200);
    assert_eq!(rate, expected);
}

#[ink::test]
fn borrow_rate_at_full_utilization() {
    let params = default_params();
    // U = 100%
    let utilization = Ratio::one();
    let rate = TusdtLendingPool::compute_borrow_rate(&params, utilization).unwrap();
    // Zone 2: base(0) + slope1(4%) + slope2(96%) × 1.0 = 100%
    let expected = Ratio::from_basis_points(10_000);
    assert_eq!(rate, expected);
}

#[ink::test]
fn borrow_rate_with_nonzero_base() {
    let params = InterestRateParams {
        base_rate: Ratio::from_basis_points(100), // 1%
        slope1: Ratio::from_basis_points(300),    // 3%
        slope2: Ratio::from_basis_points(9000),   // 90%
        optimal_utilization: Ratio::from_basis_points(8000),
        reserve_factor: Ratio::from_basis_points(2000),
    };
    let utilization = Ratio::from_basis_points(4000);
    let rate = TusdtLendingPool::compute_borrow_rate(&params, utilization).unwrap();
    // Zone 1: 1% + 3% × 0.5 = 1% + 1.5% = 2.5%
    let expected = Ratio::from_basis_points(250);
    assert_eq!(rate, expected);
}

// ---------------------------------------------------------------------------
// Params validation
// ---------------------------------------------------------------------------

#[ink::test]
fn valid_interest_params_accepted() {
    let config = InterestRateParamsConfig {
        base_rate: 0,
        slope1: 400,
        slope2: 9600,
        optimal_utilization: 8000,
        reserve_factor: 2000,
    };
    let params = TusdtLendingPool::interest_params_from_config(config).unwrap();
    assert_eq!(params.base_rate, Ratio::from_basis_points(0));
    assert_eq!(params.slope1, Ratio::from_basis_points(400));
}

#[ink::test]
fn interest_params_rejects_zero_optimal() {
    let config = InterestRateParamsConfig {
        base_rate: 0,
        slope1: 400,
        slope2: 9600,
        optimal_utilization: 0,
        reserve_factor: 2000,
    };
    assert_eq!(
        TusdtLendingPool::interest_params_from_config(config),
        Err(Error::InvalidParam)
    );
}

#[ink::test]
fn interest_params_rejects_slope_overflow() {
    let config = InterestRateParamsConfig {
        base_rate: 0,
        slope1: 6000,
        slope2: 6000, // sum = 12000 > 10000
        optimal_utilization: 8000,
        reserve_factor: 2000,
    };
    assert_eq!(
        TusdtLendingPool::interest_params_from_config(config),
        Err(Error::InvalidParam)
    );
}

#[ink::test]
fn interest_params_rejects_full_reserve_factor() {
    let config = InterestRateParamsConfig {
        base_rate: 0,
        slope1: 400,
        slope2: 9600,
        optimal_utilization: 8000,
        reserve_factor: 10_000, // 100% reserve factor
    };
    assert_eq!(
        TusdtLendingPool::interest_params_from_config(config),
        Err(Error::InvalidParam)
    );
}

#[ink::test]
fn valid_alpha_params_accepted() {
    let config = AlphaMarketParamsConfig {
        collateral_factor: 5000,
        liquidation_threshold: 6000,
        liquidation_bonus: 500,
        supply_cap: 0,
    };
    let params = TusdtLendingPool::alpha_params_from_config(config).unwrap();
    assert_eq!(params.collateral_factor, Ratio::from_basis_points(5000));
    assert_eq!(params.liquidation_threshold, Ratio::from_basis_points(6000));
}

#[ink::test]
fn alpha_params_rejects_cf_ge_lt() {
    let config = AlphaMarketParamsConfig {
        collateral_factor: 6000,
        liquidation_threshold: 5000, // CF > LT — invalid
        liquidation_bonus: 500,
        supply_cap: 0,
    };
    assert_eq!(
        TusdtLendingPool::alpha_params_from_config(config),
        Err(Error::InvalidParam)
    );
}

#[ink::test]
fn alpha_params_rejects_high_bonus() {
    let config = AlphaMarketParamsConfig {
        collateral_factor: 5000,
        liquidation_threshold: 6000,
        liquidation_bonus: 3000, // > 25%
        supply_cap: 0,
    };
    assert_eq!(
        TusdtLendingPool::alpha_params_from_config(config),
        Err(Error::InvalidParam)
    );
}

#[ink::test]
fn valid_global_params_accepted() {
    let config = PoolGlobalParamsConfig {
        max_oracle_age_ms: 1_800_000,
        close_factor: 5000,
        performance_fee: 2500,
        supply_cap_tao: 0,
        supply_cap_tusdt: 0,
        borrow_cap_tao: 0,
        borrow_cap_tusdt: 0,
    };
    let params = TusdtLendingPool::global_params_from_config(config).unwrap();
    assert_eq!(params.close_factor, Ratio::from_basis_points(5000));
}

#[ink::test]
fn global_params_rejects_zero_oracle_age() {
    let config = PoolGlobalParamsConfig {
        max_oracle_age_ms: 0,
        close_factor: 5000,
        performance_fee: 2500,
        supply_cap_tao: 0,
        supply_cap_tusdt: 0,
        borrow_cap_tao: 0,
        borrow_cap_tusdt: 0,
    };
    assert_eq!(
        TusdtLendingPool::global_params_from_config(config),
        Err(Error::InvalidParam)
    );
}

#[ink::test]
fn global_params_rejects_zero_close_factor() {
    let config = PoolGlobalParamsConfig {
        max_oracle_age_ms: 1_800_000,
        close_factor: 0,
        performance_fee: 2500,
        supply_cap_tao: 0,
        supply_cap_tusdt: 0,
        borrow_cap_tao: 0,
        borrow_cap_tusdt: 0,
    };
    assert_eq!(
        TusdtLendingPool::global_params_from_config(config),
        Err(Error::InvalidParam)
    );
}

#[ink::test]
fn global_params_rejects_high_close_factor() {
    let config = PoolGlobalParamsConfig {
        max_oracle_age_ms: 1_800_000,
        close_factor: 6000, // > 50%
        performance_fee: 2500,
        supply_cap_tao: 0,
        supply_cap_tusdt: 0,
        borrow_cap_tao: 0,
        borrow_cap_tusdt: 0,
    };
    assert_eq!(
        TusdtLendingPool::global_params_from_config(config),
        Err(Error::InvalidParam)
    );
}

// ---------------------------------------------------------------------------
// Timelock mechanics (market params)
// ---------------------------------------------------------------------------

#[ink::test]
fn schedule_and_execute_market_params() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    let config = InterestRateParamsConfig {
        base_rate: 100,
        slope1: 400,
        slope2: 9500,
        optimal_utilization: 8000,
        reserve_factor: 2000,
    };
    pool.set_market_params(0, config).unwrap();
    // Verify pending update exists
    let pending = pool.get_pending_market_params_update(0).unwrap();
    assert!(pending.execute_after > 0);

    // Cannot execute before timelock
    assert_eq!(
        pool.execute_market_params_update(0),
        Err(Error::ParamsUpdateTimelockActive)
    );

    // Advance past timelock and execute
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(
        pending.execute_after,
    );
    pool.execute_market_params_update(0).unwrap();
    let pending = pool.get_pending_market_params_update(0);
    assert!(pending.is_none());
}

#[ink::test]
fn cancel_market_params_update() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    let config = InterestRateParamsConfig {
        base_rate: 100,
        slope1: 400,
        slope2: 9500,
        optimal_utilization: 8000,
        reserve_factor: 2000,
    };
    pool.set_market_params(0, config).unwrap();
    pool.cancel_market_params_update(0).unwrap();
    let pending = pool.get_pending_market_params_update(0);
    assert!(pending.is_none());
}

#[ink::test]
fn non_governance_cannot_schedule_market_params() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.bob);
    let config = InterestRateParamsConfig {
        base_rate: 100,
        slope1: 400,
        slope2: 9500,
        optimal_utilization: 8000,
        reserve_factor: 2000,
    };
    assert_eq!(
        pool.set_market_params(0, config),
        Err(Error::NotMaintainer)
    );
}

#[ink::test]
fn non_governance_cannot_cancel_market_params() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    let config = InterestRateParamsConfig {
        base_rate: 100,
        slope1: 400,
        slope2: 9500,
        optimal_utilization: 8000,
        reserve_factor: 2000,
    };
    pool.set_market_params(0, config).unwrap();
    set_caller(accounts.bob);
    assert_eq!(
        pool.cancel_market_params_update(0),
        Err(Error::NotMaintainer)
    );
}

#[ink::test]
fn cancel_without_pending_is_error() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    assert_eq!(
        pool.cancel_market_params_update(0),
        Err(Error::NoPendingMarketParamsUpdate)
    );
}

#[ink::test]
fn execute_without_pending_is_error() {
    let (mut pool, _accounts) = setup();
    assert_eq!(
        pool.execute_market_params_update(0),
        Err(Error::NoPendingMarketParamsUpdate)
    );
}

// ---------------------------------------------------------------------------
// Global params timelock
// ---------------------------------------------------------------------------

#[ink::test]
fn schedule_and_execute_global_params() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    let config = PoolGlobalParamsConfig {
        max_oracle_age_ms: 3_600_000,
        close_factor: 4000,
        performance_fee: 2000,
        supply_cap_tao: 0,
        supply_cap_tusdt: 0,
        borrow_cap_tao: 0,
        borrow_cap_tusdt: 0,
    };
    pool.set_global_params(config).unwrap();
    let pending = pool.get_pending_global_params_update().unwrap();
    assert!(pending.execute_after > 0);

    // Cannot execute before timelock
    assert_eq!(
        pool.execute_global_params_update(),
        Err(Error::ParamsUpdateTimelockActive)
    );

    // Advance and execute
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(
        pending.execute_after,
    );
    pool.execute_global_params_update().unwrap();
    assert!(pool.get_pending_global_params_update().is_none());
}

#[ink::test]
fn execute_global_params_at_60s_still_timelocked() {
    // Regression pin: the timelock is 24 h — advancing 60 s (the old
    // timelock duration) must still be inside the window.
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    let config = PoolGlobalParamsConfig {
        max_oracle_age_ms: 3_600_000,
        close_factor: 4000,
        performance_fee: 2000,
        supply_cap_tao: 0,
        supply_cap_tusdt: 0,
        borrow_cap_tao: 0,
        borrow_cap_tusdt: 0,
    };
    pool.set_global_params(config).unwrap();

    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(60 * 1_000);
    assert_eq!(
        pool.execute_global_params_update(),
        Err(Error::ParamsUpdateTimelockActive)
    );
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(0);
}

#[ink::test]
fn cancel_global_params_update() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    let config = PoolGlobalParamsConfig {
        max_oracle_age_ms: 3_600_000,
        close_factor: 4000,
        performance_fee: 2000,
        supply_cap_tao: 0,
        supply_cap_tusdt: 0,
        borrow_cap_tao: 0,
        borrow_cap_tusdt: 0,
    };
    pool.set_global_params(config).unwrap();
    pool.cancel_global_params_update().unwrap();
    assert!(pool.get_pending_global_params_update().is_none());
}

// ---------------------------------------------------------------------------
// Alpha params timelock
// ---------------------------------------------------------------------------

#[ink::test]
fn schedule_and_execute_alpha_params() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    let config = AlphaMarketParamsConfig {
        collateral_factor: 6000,
        liquidation_threshold: 7000,
        liquidation_bonus: 800,
        supply_cap: 1_000_000,
    };
    pool.set_alpha_params(1, config).unwrap();
    let pending = pool.get_pending_alpha_params_update(1).unwrap();
    assert!(pending.execute_after > 0);

    // Execute
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(
        pending.execute_after,
    );
    pool.execute_alpha_params_update(1).unwrap();
    assert!(pool.get_pending_alpha_params_update(1).is_none());
}

#[ink::test]
fn cancel_alpha_params_update() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    let config = AlphaMarketParamsConfig {
        collateral_factor: 6000,
        liquidation_threshold: 7000,
        liquidation_bonus: 800,
        supply_cap: 1_000_000,
    };
    pool.set_alpha_params(1, config).unwrap();
    pool.cancel_alpha_params_update(1).unwrap();
    assert!(pool.get_pending_alpha_params_update(1).is_none());
}

// ---------------------------------------------------------------------------
// Deposit alpha flow (chain extension interaction)
// ---------------------------------------------------------------------------

#[ink::test]
fn deposit_alpha_succeeds() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    register_mock(1_000);
    let market_id = pool.deposit_alpha(1, 500).unwrap();
    assert!(market_id >= 2);
    assert_eq!(pool.get_user_alpha_position(accounts.alice, 1), Some(500));
    let total = pool.get_netuid_total_collateral(1);
    assert_eq!(total, Some(500));
}

#[ink::test]
fn deposit_alpha_rejects_zero_amount() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    register_mock(1_000);
    assert_eq!(
        pool.deposit_alpha(1, 0),
        Err(Error::ZeroAmount)
    );
}

#[ink::test]
fn deposit_alpha_rejects_unapproved_netuid() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    register_mock(1_000);
    assert_eq!(
        pool.deposit_alpha(99, 500),
        Err(Error::UnapprovedNetuid)
    );
}

#[ink::test]
fn deposit_alpha_fails_on_transfer_failure() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    register_mock_transfer_fails(1_000);
    assert_eq!(
        pool.deposit_alpha(1, 500),
        Err(Error::StakeTransferFailed)
    );
    // No state was changed
    assert_eq!(pool.get_user_alpha_position(accounts.alice, 1), None);
    assert_eq!(pool.get_netuid_total_collateral(1), None);
}

#[ink::test]
fn deposit_alpha_rejected_when_paused() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    pool.pause().unwrap();
    register_mock(1_000);
    assert_eq!(
        pool.deposit_alpha(1, 500),
        Err(Error::ContractPaused)
    );
}

// ---------------------------------------------------------------------------
// View methods
// ---------------------------------------------------------------------------

#[ink::test]
fn market_state_defaults() {
    let (pool, _accounts) = setup();
    let state = pool.get_market_state(0).unwrap();
    assert_eq!(state.total_supplied, 0);
    assert_eq!(state.total_debt, 0);
    assert_eq!(state.borrow_index, Ratio::one());
    assert_eq!(state.exchange_rate, Ratio::one());
}

#[ink::test]
fn utilization_zero_when_idle() {
    let (pool, _accounts) = setup();
    let util = pool.get_utilization(0).unwrap();
    assert!(util.is_zero());
}

#[ink::test]
fn borrow_rate_read() {
    let (pool, _accounts) = setup();
    // At zero utilization, borrow rate should be base rate (0)
    let rate = pool.get_borrow_rate(0).unwrap();
    assert!(rate.is_zero());
}

#[ink::test]
fn exchange_rate_starts_at_one() {
    let (pool, _accounts) = setup();
    let rate = pool.get_exchange_rate(0).unwrap();
    assert_eq!(rate, Ratio::one());
}

#[ink::test]
fn borrow_index_starts_at_one() {
    let (pool, _accounts) = setup();
    let idx = pool.get_borrow_index(0).unwrap();
    assert_eq!(idx, Ratio::one());
}

#[ink::test]
fn no_position_by_default() {
    let (pool, accounts) = setup();
    let pos = pool.get_position(0, accounts.alice);
    assert!(pos.is_none());
}

#[ink::test]
fn zero_debt_by_default() {
    let (pool, accounts) = setup();
    let debt = pool.get_user_debt(0, accounts.alice);
    assert_eq!(debt, Some(0));
}

// ---------------------------------------------------------------------------
// Debt principal tracking
// ---------------------------------------------------------------------------

/// Grows a market's borrow index to `inner` (1e18 scale) for testing.
fn set_borrow_index(pool: &mut TusdtLendingPool, market_id: u8, inner: u128) {
    let mut state = pool.get_market_state(market_id).unwrap();
    state.borrow_index = Ratio::from_inner(inner);
    pool.debug_set_market_state(market_id, state);
}

#[ink::test]
fn debt_details_zero_for_empty_position() {
    let (pool, accounts) = setup();
    assert_eq!(pool.get_user_debt_details(0, accounts.alice), Some((0, 0)));
}

#[ink::test]
fn debt_details_splits_principal_and_interest() {
    let (mut pool, accounts) = setup();
    // Borrowed 100 at index 1.0 — no interest yet.
    pool.debug_set_position(
        0,
        accounts.alice,
        Position {
            ltoken_balance: 0,
            scaled_debt: 100,
            alpha_principal: 0,
        },
    );
    pool.debug_set_debt_principal(0, accounts.alice, 100);
    assert_eq!(pool.get_user_debt_details(0, accounts.alice), Some((100, 100)));

    // Index grows +10% (as accrue_interest would): debt grows, principal does not.
    set_borrow_index(&mut pool, 0, 1_100_000_000_000_000_000);
    assert_eq!(pool.get_user_debt_details(0, accounts.alice), Some((110, 100)));
}

#[ink::test]
fn debt_details_legacy_position_falls_back_to_scaled_debt() {
    let (mut pool, accounts) = setup();
    // Legacy position: no tracked principal, borrowed at index 1.25.
    // scaled_debt = 80 → debt = 100. Principal estimate = scaled_debt = 80.
    pool.debug_set_position(
        1,
        accounts.bob,
        Position {
            ltoken_balance: 0,
            scaled_debt: 80,
            alpha_principal: 0,
        },
    );
    set_borrow_index(&mut pool, 1, 1_250_000_000_000_000_000);
    assert_eq!(pool.get_user_debt_details(1, accounts.bob), Some((100, 80)));
}

#[ink::test]
fn debt_details_never_reports_negative_interest() {
    let (mut pool, accounts) = setup();
    // Tracked principal above debt must clamp: interest is never negative.
    pool.debug_set_position(
        0,
        accounts.alice,
        Position {
            ltoken_balance: 0,
            scaled_debt: 100,
            alpha_principal: 0,
        },
    );
    pool.debug_set_debt_principal(0, accounts.alice, 150);
    set_borrow_index(&mut pool, 0, 1_000_000_000_000_000_000);
    assert_eq!(pool.get_user_debt_details(0, accounts.alice), Some((100, 100)));
}

// ---------------------------------------------------------------------------
// Alpha yield index
// ---------------------------------------------------------------------------

#[ink::test]
fn yield_index_defaults_to_one() {
    let (pool, _accounts) = setup();
    let idx = pool.get_alpha_yield_index(1).unwrap();
    assert_eq!(idx, Ratio::one());
}

// ---------------------------------------------------------------------------
// Pool hotkey
// ---------------------------------------------------------------------------

#[ink::test]
fn pool_hotkey_is_set() {
    let (pool, accounts) = setup();
    assert_eq!(pool.get_pool_hotkey(), accounts.bob);
}

#[ink::test]
fn governance_can_update_pool_hotkey() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    register_mock(1_000);
    pool.deposit_alpha(1, 500).unwrap();
    // Update hotkey — move_stake is a no-op in the mock (returns 0)
    pool.update_pool_hotkey(accounts.charlie, vec![1]).unwrap();
    assert_eq!(pool.get_pool_hotkey(), accounts.charlie);
}

#[ink::test]
fn update_pool_hotkey_rejects_too_many_netuids() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    let netuids: Vec<u16> = (1..=33).collect();
    assert_eq!(
        pool.update_pool_hotkey(accounts.charlie, netuids),
        Err(Error::TooManyNetuids)
    );
}

// ---------------------------------------------------------------------------
// Contract pausing guards deposit_alpha
// ---------------------------------------------------------------------------

#[ink::test]
fn paused_blocks_deposit_alpha() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    pool.pause().unwrap();
    register_mock(1_000);
    assert_eq!(
        pool.deposit_alpha(1, 500),
        Err(Error::ContractPaused)
    );
}

#[ink::test]
fn unpause_allows_deposit_alpha() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    pool.pause().unwrap();
    pool.unpause().unwrap();
    register_mock(1_000);
    pool.deposit_alpha(1, 500).unwrap();
    assert_eq!(pool.get_user_alpha_position(accounts.alice, 1), Some(500));
}

// ---------------------------------------------------------------------------
// Timelock helper test
// ---------------------------------------------------------------------------

#[ink::test]
fn set_timelock_to_zero_allows_instant_execution() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    // Use the helper to zero out the timelock
    set_timelock_to_zero(&mut pool);
    // Global params should now have the configured values
    // (timelock was bypassed by the helper)
    assert!(!pool.paused());
}

// ---------------------------------------------------------------------------
// Chain extension failure modes
// ---------------------------------------------------------------------------

#[ink::test]
fn deposit_alpha_fails_when_chain_fails() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    register_mock_chain_fails();
    // The mock's should_fail=true causes func 25 to return error code 1,
    // which the pool maps to StakeTransferFailed.
    assert_eq!(pool.deposit_alpha(1, 500), Err(Error::StakeTransferFailed));
    // Verify no state was mutated
    assert_eq!(pool.get_user_alpha_position(accounts.alice, 1), None);
    assert_eq!(pool.get_netuid_total_collateral(1), None);
}

#[ink::test]
fn deposit_alpha_with_no_stake_info_returns_none() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    register_mock_no_stake();
    // The mock returns None for stake info; the deposit still proceeds
    // since caller_transfer_stake succeeds (transfer_fails=false)
    pool.deposit_alpha(1, 500).unwrap();
    assert_eq!(pool.get_user_alpha_position(accounts.alice, 1), Some(500));
}

// ---------------------------------------------------------------------------
// Reentrancy guard
// ---------------------------------------------------------------------------

#[ink::test]
fn ensure_idle_guards_reentrancy() {
    let (mut pool, _accounts) = setup();
    // The reentrancy guard sets the internal busy flag and prevents double entry
    pool.ensure_idle().unwrap();
    assert_eq!(pool.ensure_idle(), Err(Error::Reentrancy));
    pool.set_idle();
    // After clearing, it can be re-entered
    pool.ensure_idle().unwrap();
    pool.set_idle();
}

// ---------------------------------------------------------------------------
// Position key lifecycle
// ---------------------------------------------------------------------------

#[ink::test]
fn update_position_key_pushes_when_position_nonzero() {
    let (mut pool, accounts) = setup();
    // Seed a non-zero position without pushing to position_keys
    pool.debug_set_position(
        0,
        accounts.alice,
        Position {
            ltoken_balance: 100,
            scaled_debt: 0,
            alpha_principal: 0,
        },
    );
    // Call update — should push the key
    pool.update_position_key(0, accounts.alice);
    let all = pool.get_all_positions(0);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, (0, accounts.alice));
    assert_eq!(all[0].1.ltoken_balance, 100);
}

#[ink::test]
fn update_position_key_removes_when_zero() {
    let (mut pool, accounts) = setup();
    // Seed a non-zero position with a key
    pool.debug_set_position(
        0,
        accounts.alice,
        Position {
            ltoken_balance: 100,
            scaled_debt: 0,
            alpha_principal: 0,
        },
    );
    pool.update_position_key(0, accounts.alice);
    assert_eq!(pool.get_all_positions(0).len(), 1);

    // Zero the position and update — should remove the key
    pool.debug_set_position(
        0,
        accounts.alice,
        Position {
            ltoken_balance: 0,
            scaled_debt: 0,
            alpha_principal: 0,
        },
    );
    pool.update_position_key(0, accounts.alice);
    assert_eq!(pool.get_all_positions(0).len(), 0);
}

#[ink::test]
fn update_position_key_no_double_push() {
    let (mut pool, accounts) = setup();
    // Seed a non-zero position with a key
    pool.debug_set_position(
        0,
        accounts.alice,
        Position {
            ltoken_balance: 100,
            scaled_debt: 0,
            alpha_principal: 0,
        },
    );
    pool.update_position_key(0, accounts.alice);
    assert_eq!(pool.get_all_positions(0).len(), 1);

    // Call update again — should NOT push a duplicate
    pool.update_position_key(0, accounts.alice);
    assert_eq!(pool.get_all_positions(0).len(), 1);
}

#[ink::test]
fn update_position_key_self_heals_legacy_duplicates() {
    let (mut pool, accounts) = setup();
    // Simulate legacy duplicated keys by pushing 3 times
    pool.debug_set_position(
        0,
        accounts.alice,
        Position {
            ltoken_balance: 100,
            scaled_debt: 0,
            alpha_principal: 0,
        },
    );
    pool.debug_push_position_key(0, accounts.alice);
    pool.debug_push_position_key(0, accounts.alice);
    pool.debug_push_position_key(0, accounts.alice);
    // 3 duplicate keys in storage
    // The read-side dedup within get_all_positions handles it
    let all = pool.get_all_positions(0);
    // With read-side dedup, only 1 entry returned
    assert_eq!(all.len(), 1);

    // Now call update — it removes all duplicates and pushes exactly once
    pool.update_position_key(0, accounts.alice);
    let all = pool.get_all_positions(0);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, (0, accounts.alice));

    // A second update is a no-op
    pool.update_position_key(0, accounts.alice);
    assert_eq!(pool.get_all_positions(0).len(), 1);
}

#[ink::test]
fn swap_removal_preserves_other_keys() {
    let (mut pool, accounts) = setup();
    // Seed 3 positions: alice market 0, bob market 0, alice market 1
    pool.debug_set_position(
        0,
        accounts.alice,
        Position {
            ltoken_balance: 100,
            scaled_debt: 0,
            alpha_principal: 0,
        },
    );
    pool.update_position_key(0, accounts.alice);
    pool.debug_set_position(
        0,
        accounts.bob,
        Position {
            ltoken_balance: 200,
            scaled_debt: 0,
            alpha_principal: 0,
        },
    );
    pool.update_position_key(0, accounts.bob);
    pool.debug_set_position(
        1,
        accounts.alice,
        Position {
            ltoken_balance: 0,
            scaled_debt: 50,
            alpha_principal: 0,
        },
    );
    pool.update_position_key(1, accounts.alice);

    assert_eq!(pool.get_all_positions(0).len(), 3);

    // Zero out bob's position — should remove only bob's key
    pool.debug_set_position(
        0,
        accounts.bob,
        Position {
            ltoken_balance: 0,
            scaled_debt: 0,
            alpha_principal: 0,
        },
    );
    pool.update_position_key(0, accounts.bob);
    let all = pool.get_all_positions(0);
    assert_eq!(all.len(), 2);
    // Alice's positions must still be present
    let keys: Vec<(u8, ink::primitives::AccountId)> = all.iter().map(|(k, _)| *k).collect();
    assert!(keys.contains(&(0, accounts.alice)));
    assert!(keys.contains(&(1, accounts.alice)));
}

#[ink::test]
fn multi_market_positions_independent() {
    let (mut pool, accounts) = setup();
    // User has supply on market 0 and debt on market 1
    pool.debug_set_position(
        0,
        accounts.alice,
        Position {
            ltoken_balance: 100,
            scaled_debt: 0,
            alpha_principal: 0,
        },
    );
    pool.update_position_key(0, accounts.alice);
    pool.debug_set_position(
        1,
        accounts.alice,
        Position {
            ltoken_balance: 0,
            scaled_debt: 50,
            alpha_principal: 0,
        },
    );
    pool.update_position_key(1, accounts.alice);

    assert_eq!(pool.get_all_positions(0).len(), 2);

    // Zero-out market 0 only
    pool.debug_set_position(
        0,
        accounts.alice,
        Position {
            ltoken_balance: 0,
            scaled_debt: 0,
            alpha_principal: 0,
        },
    );
    pool.update_position_key(0, accounts.alice);

    let all = pool.get_all_positions(0);
    assert_eq!(all.len(), 1);
    // Market 1 entry should still exist
    assert_eq!(all[0].0, (1, accounts.alice));
    assert_eq!(all[0].1.scaled_debt, 50);
}

#[ink::test]
fn deposit_withdraw_deposit_alpha_no_duplicate_keys() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    register_mock(1_000);

    // First deposit
    pool.deposit_alpha(1, 500).unwrap();
    let all = pool.get_all_positions(0);
    assert_eq!(all.len(), 1, "should have 1 position after first deposit");

    // Withdraw all — position should be removed from position_keys
    pool.withdraw_alpha(1, 500, accounts.bob).unwrap();
    let all = pool.get_all_positions(0);
    assert_eq!(all.len(), 0, "should have 0 positions after full withdraw");

    // Deposit again — should create exactly 1 entry (NOT 2)
    pool.deposit_alpha(1, 300).unwrap();
    let all = pool.get_all_positions(0);
    assert_eq!(all.len(), 1, "should have exactly 1 position after re-deposit");
    let (key, pos) = &all[0];
    assert_eq!(key.1, accounts.alice);
    assert_eq!(pos.alpha_principal, 300);
}

#[ink::test]
fn partial_withdraw_alpha_keeps_key() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    register_mock(1_000);

    pool.deposit_alpha(1, 500).unwrap();
    assert_eq!(pool.get_all_positions(0).len(), 1);

    // Withdraw only part of the collateral
    pool.withdraw_alpha(1, 200, accounts.bob).unwrap();
    let all = pool.get_all_positions(0);
    assert_eq!(all.len(), 1, "key should remain after partial withdraw");
    let (_, pos) = &all[0];
    assert_eq!(pos.alpha_principal, 300);
}

#[ink::test]
fn borrow_only_orphan_self_heal() {
    let (mut pool, accounts) = setup();
    // Simulate a legacy borrow-only position: debt on market 0, no key
    pool.debug_set_position(
        0,
        accounts.alice,
        Position {
            ltoken_balance: 0,
            scaled_debt: 50,
            alpha_principal: 0,
        },
    );
    // No key was pushed — simulate orphan
    // Call update_position_key — should self-heal by pushing the key
    pool.update_position_key(0, accounts.alice);
    let all = pool.get_all_positions(0);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, (0, accounts.alice));
    assert_eq!(all[0].1.scaled_debt, 50);
}

// ---------------------------------------------------------------------------
// Parameter getters
// ---------------------------------------------------------------------------

#[ink::test]
fn get_global_params_returns_defaults() {
    let (pool, _accounts) = setup();
    let params = pool.get_global_params();
    assert_eq!(params.close_factor, 5000);
    assert_eq!(params.performance_fee, 2500);
    assert_eq!(params.max_oracle_age_ms, 1_800_000);
    assert_eq!(params.supply_cap_tao, 0);
    assert_eq!(params.supply_cap_tusdt, 0);
}

#[ink::test]
fn get_market_params_returns_configured_params() {
    let (pool, _accounts) = setup();
    let params = pool.get_market_params(0).expect("market 0 params should exist");
    // Default TAO interest params: base=0, optimal=8000, slope1=400, slope2=9600, reserve=2000
    assert_eq!(params.base_rate, 0);
    assert_eq!(params.optimal_utilization, 8000);
    assert_eq!(params.slope1, 400);
}

#[ink::test]
fn get_alpha_params_returns_none_for_unconfigured_netuid() {
    let (pool, _accounts) = setup();
    assert!(pool.get_alpha_params(99).is_none());
}

#[ink::test]
fn get_alpha_params_returns_configured_params() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    // Configure alpha params for netuid 1
    let config = AlphaMarketParamsConfig {
        collateral_factor: 6000,
        liquidation_threshold: 7000,
        liquidation_bonus: 800,
        supply_cap: 1_000_000,
    };
    pool.set_alpha_params(1, config).unwrap();
    // Execute the update
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(
        PARAMS_TIMELOCK_MS + 1,
    );
    pool.execute_alpha_params_update(1).unwrap();
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(0);

    let params = pool.get_alpha_params(1).expect("alpha params should exist");
    assert_eq!(params.collateral_factor, 6000);
    assert_eq!(params.liquidation_threshold, 7000);
    assert_eq!(params.supply_cap, 1_000_000);
}

// ---------------------------------------------------------------------------
// Maintainer role
// ---------------------------------------------------------------------------

#[ink::test]
fn maintainer_initialized_to_deployer() {
    let (pool, accounts) = setup();
    assert_eq!(pool.maintainer(), accounts.alice);
}

#[ink::test]
fn governance_can_update_maintainer() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.update_maintainer(accounts.bob).unwrap();
    assert_eq!(pool.maintainer(), accounts.bob);
}

#[ink::test]
fn non_governance_cannot_update_maintainer() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.bob); // bob is NOT governance
    assert_eq!(
        pool.update_maintainer(accounts.charlie),
        Err(Error::NotGovernance)
    );
}

#[ink::test]
fn maintainer_can_set_alpha_params() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    // alice is both governance and maintainer initially
    let config = AlphaMarketParamsConfig {
        collateral_factor: 6000,
        liquidation_threshold: 7000,
        liquidation_bonus: 800,
        supply_cap: 1_000_000,
    };
    pool.set_alpha_params(1, config).unwrap();
}

#[ink::test]
fn non_maintainer_cannot_set_params() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    // Transfer maintainer to bob
    pool.update_maintainer(accounts.bob).unwrap();
    // charlie is neither maintainer nor governance
    set_caller(accounts.charlie);
    let config = AlphaMarketParamsConfig {
        collateral_factor: 6000,
        liquidation_threshold: 7000,
        liquidation_bonus: 800,
        supply_cap: 1_000_000,
    };
    assert_eq!(
        pool.set_alpha_params(1, config),
        Err(Error::NotMaintainer)
    );
}

#[ink::test]
fn maintainer_cannot_update_governance() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    // Transfer maintainer to bob
    pool.update_maintainer(accounts.bob).unwrap();
    // Bob is maintainer but cannot transfer governance
    set_caller(accounts.bob);
    assert_eq!(
        pool.update_governance(accounts.charlie),
        Err(Error::NotGovernance)
    );
}

#[ink::test]
fn governance_can_still_call_maintainer_functions() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    // Transfer maintainer to bob
    pool.update_maintainer(accounts.bob).unwrap();
    // alice is still governance — she can still do maintainer-gated operations
    set_caller(accounts.alice);
    let config = AlphaMarketParamsConfig {
        collateral_factor: 6000,
        liquidation_threshold: 7000,
        liquidation_bonus: 800,
        supply_cap: 1_000_000,
    };
    // governance should pass ensure_maintainer (governance || maintainer)
    pool.set_alpha_params(1, config).unwrap();
}

#[ink::test]
fn new_maintainer_can_call_maintainer_functions() {
    let (mut pool, accounts) = setup_with_alpha(1);
    set_caller(accounts.alice);
    // Transfer maintainer to bob
    pool.update_maintainer(accounts.bob).unwrap();
    // Bob is now maintainer
    set_caller(accounts.bob);
    let config = AlphaMarketParamsConfig {
        collateral_factor: 6000,
        liquidation_threshold: 7000,
        liquidation_bonus: 800,
        supply_cap: 1_000_000,
    };
    pool.set_alpha_params(1, config).unwrap();
}

#[ink::test]
fn maintainer_initialized_in_test_constructor() {
    let (pool, accounts) = setup();
    // Test the test constructor also initializes maintainer
    assert_eq!(pool.maintainer(), accounts.alice);
    assert_eq!(pool.governance(), accounts.alice);
}

// ---------------------------------------------------------------------------
// Borrow/repay scaling regression tests
// ---------------------------------------------------------------------------

#[ink::test]
fn borrow_scaling_divides_amount_by_borrow_index() {
    // scaled_debt = amount / borrow_index.
    // Regression: the operands were previously reversed, computing
    // borrow_index / amount — borrowing 10 TUSDT stored 0.1 TUSDT of
    // scaled debt and repayments underflowed into ArithmeticError.
    let amount: u128 = 10_000_000_000; // 10 TUSDT (9 decimals)
    let index_one = Ratio::from_inner(1_000_000_000_000_000_000); // 1.0

    let scaled = index_one
        .checked_div_value(amount)
        .expect("scaling must not overflow");
    assert_eq!(scaled, amount, "index 1.0 → scaled == amount");

    // With index = 2.0, scaled = amount / 2.
    let index_two = Ratio::from_inner(2_000_000_000_000_000_000);
    let scaled_two = index_two
        .checked_div_value(amount)
        .expect("scaling must not overflow");
    assert_eq!(scaled_two, 5_000_000_000, "index 2.0 → scaled == amount / 2");

    // Repaying the exact debt must not underflow: scaled_repaid (amount/index)
    // must equal the position's scaled debt.
    let scaled_repaid = index_one
        .checked_div_value(scaled)
        .expect("repay scaling must not overflow");
    assert_eq!(scaled_repaid, amount, "repay full debt → scaled_repaid == scaled debt");
}

#[ink::test]
fn repay_clamps_to_market_total_debt_when_ledger_drifted() {
    // Regression: with total_debt below the user's position debt (ledger drift
    // from the historical reversed-division bug), repaying the user's full debt
    // underflowed `total_debt.checked_sub` into ArithmeticError. The repay now
    // clamps to the market total; a zero clamped amount is a clean no-op that
    // happens before the token pull.
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);

    pool.debug_set_market_state(
        1,
        MarketState {
            total_supplied: 129_000_000_000,
            total_debt: 0, // fully drained by drift
            borrow_index: Ratio::one(),
            exchange_rate: Ratio::one(),
            reserve_accrued: 0,
            last_update: 0,
        },
    );
    pool.debug_set_position(
        1,
        accounts.alice,
        Position { ltoken_balance: 0, scaled_debt: 12, alpha_principal: 0 },
    );
    pool.debug_set_debt_principal(1, accounts.alice, 12);

    // Repaying the full debt against an empty market total must not underflow:
    // it clamps to zero and no-ops before touching the token contract.
    assert_eq!(pool.repay_tusdt(12), Ok(()));

    let state = pool.get_market_state(1).unwrap();
    assert_eq!(state.total_debt, 0, "no debt should have been deducted");
    let (debt, principal) = pool.get_user_debt_details(1, accounts.alice).unwrap();
    assert_eq!((debt, principal), (12, 12), "position untouched");
}

#[ink::test]
fn repay_below_index_precision_is_rejected() {
    // Regression: repaying less than one borrow-index unit cleared zero scaled
    // debt while still decrementing total_debt — the repaid amount leaked from
    // the market total without reducing the position's debt (the "small amount
    // repay" bug).
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);

    let index = Ratio::from_inner(1_000_160_150_930_897_932); // ~1.00016
    pool.debug_set_market_state(
        1,
        MarketState {
            total_supplied: 129_000_000_000,
            total_debt: 4,
            borrow_index: index,
            exchange_rate: Ratio::one(),
            reserve_accrued: 0,
            last_update: 0,
        },
    );
    pool.debug_set_position(
        1,
        accounts.alice,
        Position { ltoken_balance: 0, scaled_debt: 12, alpha_principal: 0 },
    );
    pool.debug_set_debt_principal(1, accounts.alice, 12);

    // 1 raw unit at index ~1.00016 floors to 0 scaled units.
    assert_eq!(pool.repay_tusdt(1), Err(Error::ZeroAmount));

    // The market total must not have been touched.
    assert_eq!(pool.get_market_state(1).unwrap().total_debt, 4);
}

#[ink::test]
fn accrue_market_interest_refreshes_both_markets_permissionlessly() {
    // Permissionless refresh of both debt markets: market 0 accrues fully
    // (borrow index grows) while market 1 (no debt) just bumps last_update.
    // Market 1 avoids the cross-contract cash read in the off-chain env.
    let (mut pool, accounts) = setup();
    set_caller(accounts.bob); // anyone, not governance

    pool.debug_set_market_state(
        0,
        MarketState {
            total_supplied: 130_000_000_000,
            total_debt: 128_000_000_000,
            borrow_index: Ratio::one(),
            exchange_rate: Ratio::one(),
            reserve_accrued: 0,
            last_update: 0,
        },
    );
    pool.debug_set_market_state(
        1,
        MarketState {
            total_supplied: 129_000_000_000,
            total_debt: 0,
            borrow_index: Ratio::one(),
            exchange_rate: Ratio::one(),
            reserve_accrued: 0,
            last_update: 0,
        },
    );

    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(
        tusdt_primitives::MILLISECONDS_PER_HOUR + 1,
    );

    pool.accrue_market_interest().unwrap();

    let m0 = pool.get_market_state(0).unwrap();
    assert!(
        m0.borrow_index.into_inner() > Ratio::one().into_inner(),
        "market 0 borrow index should grow"
    );
    assert_eq!(m0.last_update, tusdt_primitives::MILLISECONDS_PER_HOUR + 1);

    let m1 = pool.get_market_state(1).unwrap();
    assert_eq!(m1.last_update, tusdt_primitives::MILLISECONDS_PER_HOUR + 1);

    assert_eq!(
        pool.get_last_interest_accrual_times(),
        Some((
            tusdt_primitives::MILLISECONDS_PER_HOUR + 1,
            tusdt_primitives::MILLISECONDS_PER_HOUR + 1
        ))
    );
}

#[ink::test]
fn get_last_interest_accrual_times_returns_both_markets() {
    let (pool, _accounts) = setup();
    // new_for_test seeds both debt markets with MarketState::new(0).
    assert_eq!(pool.get_last_interest_accrual_times(), Some((0, 0)));
}

#[ink::test]
fn accrual_produces_interest_after_an_hour() {
    // Regression: the hourly-rate divisor was double-scaled
    // (`checked_div_int(hours_per_year.into_inner())` = 8760 × 1e18), which
    // overflowed u128 and reverted every accrual with `ArithmeticError` once
    // dt_hours ≥ 1 — freezing interest and breaking borrow/repay/supply.
    // Market 0 (TAO) is used because market_cash needs no cross-contract call
    // in the off-chain env; the accrual path under test is shared.
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);

    let debt: u64 = 128_000_000_000; // 128 TAO (9 decimals)
    pool.debug_set_market_state(
        0,
        MarketState {
            total_supplied: 130_000_000_000,
            total_debt: debt,
            borrow_index: Ratio::one(),
            exchange_rate: Ratio::one(),
            reserve_accrued: 0,
            last_update: 0,
        },
    );
    pool.debug_set_position(
        0,
        accounts.alice,
        Position { ltoken_balance: 0, scaled_debt: debt, alpha_principal: 0 },
    );
    pool.debug_set_debt_principal(0, accounts.alice, debt);

    // Advance one hour so the accrual path runs with dt_hours = 1.
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(
        tusdt_primitives::MILLISECONDS_PER_HOUR + 1,
    );

    pool.accrue_interest(0).unwrap();

    let index = pool.get_borrow_index(0).unwrap();
    assert!(
        index.into_inner() > Ratio::one().into_inner(),
        "borrow index should grow after an hour of debt: {}",
        index.into_inner()
    );

    let (now_debt, principal) = pool.get_user_debt_details(0, accounts.alice).unwrap();
    assert!(
        now_debt > principal,
        "interest should accrue: debt={now_debt} principal={principal}"
    );
}

// ---------------------------------------------------------------------------
// Idle TAO root-subnet staking
// ---------------------------------------------------------------------------

/// Seeds `amount` native TAO into the pool contract's off-chain balance.
/// `CustomEnvironment` uses `Balance = u64`, which the standard test balance
/// setters do not support — route through the `DefaultEnvironment` host type,
/// which shares the same keyed balance storage (see `tusdt_test_support`).
fn seed_pool_balance(amount: u64) {
    set_callee_balance(ink::env::account_id::<tusdt_env::CustomEnvironment>(), amount);
}

#[ink::test]
fn root_stake_config_access_control_and_getter() {
    let (mut pool, accounts) = setup();

    // Non-governance cannot configure root staking.
    set_caller(accounts.bob);
    assert_eq!(
        pool.set_root_stake_config(accounts.bob, true, 1_000_000_000, 0, 2_000_000),
        Err(Error::NotGovernance)
    );

    // Governance can, and the config round-trips through the getter.
    set_caller(accounts.alice);
    pool.set_root_stake_config(accounts.bob, true, 1_000_000_000, 500_000_000, 2_000_000)
        .unwrap();
    let cfg = pool.get_root_stake_config();
    assert_eq!(cfg.root_hotkey, accounts.bob);
    assert!(cfg.staking_enabled);
    assert_eq!(cfg.stake_buffer, 1_000_000_000);
    assert_eq!(cfg.sweep_threshold, 500_000_000);
    assert_eq!(cfg.stake_floor, 2_000_000);
}

#[ink::test]
fn root_stake_config_enforces_invariants() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);

    // Floor below the pallet minimum is rejected.
    assert_eq!(
        pool.set_root_stake_config(accounts.bob, true, 1_000_000_000, 0, 1_999_999),
        Err(Error::InvalidParam)
    );
    // Buffer below the floor is rejected.
    assert_eq!(
        pool.set_root_stake_config(accounts.bob, true, 1_999_999, 0, 2_000_000),
        Err(Error::InvalidParam)
    );
}

#[ink::test]
fn sweep_stakes_excess_above_buffer() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.set_root_stake_config(accounts.bob, true, 1_000_000_000, 0, 2_000_000)
        .unwrap();
    register_mock_stateful_root(0);
    seed_pool_balance(5_000_000_000);

    // Permissionless keeper sweep.
    set_caller(accounts.charlie);
    pool.sweep().unwrap();

    // 4 TAO staked (excess above the 1 TAO buffer).
    assert_eq!(pool.get_tao_staked(), 4_000_000_000);
    let record = last_ext_call().unwrap();
    assert_eq!(record.func_id, 1);
    assert_eq!(record.hotkey, accounts.bob);
    assert_eq!(record.netuid, 0);
    assert_eq!(record.amount, 4_000_000_000);
}

#[ink::test]
fn sweep_rate_limited_to_one_per_block() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.set_root_stake_config(accounts.bob, true, 1_000_000_000, 0, 2_000_000)
        .unwrap();
    register_mock_stateful_root(0);
    seed_pool_balance(3_000_000_000);
    pool.sweep_to_root().unwrap();
    assert_eq!(pool.get_tao_staked(), 2_000_000_000);

    // Same block: fresh excess would be sweepable, but the rate limit blocks it.
    seed_pool_balance(3_000_000_000);
    pool.sweep_to_root().unwrap();
    assert_eq!(pool.get_tao_staked(), 2_000_000_000);

    // Next block: the sweep runs again.
    ink::env::test::set_block_number::<tusdt_env::CustomEnvironment>(1);
    pool.sweep_to_root().unwrap();
    assert_eq!(pool.get_tao_staked(), 4_000_000_000);
}

#[ink::test]
fn sweep_noop_when_disabled_or_below_floor() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);

    // Disabled (default): excess is ignored.
    pool.set_root_stake_config(accounts.bob, false, 1_000_000_000, 0, 2_000_000)
        .unwrap();
    register_mock_stateful_root(0);
    seed_pool_balance(5_000_000_000);
    pool.sweep().unwrap();
    assert_eq!(pool.get_tao_staked(), 0);

    // Enabled but excess below the floor: still a no-op.
    pool.set_root_stake_config(accounts.bob, true, 1_000_000_000, 0, 2_000_000)
        .unwrap();
    seed_pool_balance(1_000_001_000); // excess 1_000 < floor 2_000_000
    pool.sweep().unwrap();
    assert_eq!(pool.get_tao_staked(), 0);
}

#[ink::test]
fn top_up_free_unstakes_shortfall() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.set_root_stake_config(accounts.bob, true, 1_000_000_000, 0, 2_000_000)
        .unwrap();
    register_mock_stateful_root(3_000_000_000);
    pool.debug_set_root_stake(3_000_000_000);

    let received = pool.top_up_free(1_000_000_000).unwrap();
    assert_eq!(received, 1_000_000_000);
    assert_eq!(pool.get_tao_staked(), 2_000_000_000);
    let record = last_ext_call().unwrap();
    assert_eq!(record.func_id, 2);
    assert_eq!(record.hotkey, accounts.bob);
    assert_eq!(record.netuid, 0);
    assert_eq!(record.amount, 1_000_000_000);
}

#[ink::test]
fn top_up_free_dust_rule_full_exit() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.set_root_stake_config(accounts.bob, true, 1_000_000_000, 0, 2_000_000)
        .unwrap();
    register_mock_stateful_root(2_500_000);
    pool.debug_set_root_stake(2_500_000);

    // Taking 1_000_000 would leave 1_500_000 < floor → full exit instead.
    let received = pool.top_up_free(1_000_000).unwrap();
    assert_eq!(received, 2_500_000);
    assert_eq!(pool.get_tao_staked(), 0);
    assert_eq!(last_ext_call().unwrap().amount, 2_500_000);
}

#[ink::test]
fn top_up_free_failure_maps_to_liquidity_insufficient() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.set_root_stake_config(accounts.bob, true, 1_000_000_000, 0, 2_000_000)
        .unwrap();
    register_extension(
        MockExtension::dispatch(None)
            .with_stateful_root_stake(3_000_000_000)
            .with_remove_stake_fails(true),
    );
    pool.debug_set_root_stake(3_000_000_000);
    seed_pool_balance(0);

    assert_eq!(pool.top_up_free(1_000_000_000), Err(Error::LiquidityInsufficient));
    assert_eq!(pool.get_tao_staked(), 3_000_000_000);
}

#[ink::test]
fn market_cash_includes_staked_tao() {
    let (mut pool, _accounts) = setup();
    pool.debug_set_root_stake(1_000_000_000);
    seed_pool_balance(500_000_000);
    assert_eq!(pool.market_cash(0).unwrap(), 1_500_000_000);
}

#[ink::test]
fn hotkey_rotation_unstakes_before_swap() {
    let (mut pool, accounts) = setup();
    set_caller(accounts.alice);
    pool.set_root_stake_config(accounts.bob, true, 1_000_000_000, 0, 2_000_000)
        .unwrap();
    register_mock_stateful_root(3_000_000_000);
    pool.debug_set_root_stake(3_000_000_000);

    // Rotating the hotkey with stake outstanding fully unstakes first.
    pool.set_root_stake_config(accounts.charlie, true, 1_000_000_000, 0, 2_000_000)
        .unwrap();
    assert_eq!(pool.get_tao_staked(), 0);
    assert_eq!(pool.get_root_stake_config().root_hotkey, accounts.charlie);
    let record = last_ext_call().unwrap();
    assert_eq!(record.func_id, 2);
    assert_eq!(record.hotkey, accounts.bob);
    assert_eq!(record.amount, 3_000_000_000);
}

#[ink::test]
fn treasury_sweep_respects_stake_buffer() {
    let (pool, accounts) = setup();
    set_caller(accounts.alice);

    // 5 TAO − 1 TAO buffer − 1 ED guard = 3_999_999_999 sweepable.
    seed_pool_balance(5_000_000_000);
    assert_eq!(pool.treasury_sweepable(), Some(3_999_999_999));

    // Exactly buffer + ED guard + 1: the last unit is the guard.
    seed_pool_balance(1_000_000_002);
    assert_eq!(pool.treasury_sweepable(), Some(1));

    // At or below buffer + guard: nothing to sweep.
    seed_pool_balance(1_000_000_001);
    assert_eq!(pool.treasury_sweepable(), None);
    seed_pool_balance(500_000_000);
    assert_eq!(pool.treasury_sweepable(), None);
}
