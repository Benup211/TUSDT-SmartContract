use super::vault::*;
use ink::env::test;
use tusdt_env::StakeInfo;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_accounts() -> test::DefaultAccounts<tusdt_env::CustomEnvironment> {
    test::default_accounts::<tusdt_env::CustomEnvironment>()
}

fn set_caller(caller: ink::primitives::AccountId) {
    let callee = ink::env::account_id::<tusdt_env::CustomEnvironment>();
    ink::env::test::set_callee::<tusdt_env::CustomEnvironment>(callee);
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(caller);
}

fn setup() -> (TusdtVaultAlpha, test::DefaultAccounts<tusdt_env::CustomEnvironment>) {
    let accounts = default_accounts();
    let vault = TusdtVaultAlpha::new_for_test(accounts.alice);
    (vault, accounts)
}

fn setup_with_approved_netuid(
) -> (TusdtVaultAlpha, test::DefaultAccounts<tusdt_env::CustomEnvironment>) {
    let accounts = default_accounts();
    let mut vault = TusdtVaultAlpha::new_for_test(accounts.alice);
    set_caller(accounts.alice);
    vault.set_approved_netuid(1, true).unwrap();
    // Disable vault creation fee for tests.
    set_vault_creation_fee_to_zero(&mut vault);
    (vault, accounts)
}

fn set_vault_creation_fee_to_zero(vault: &mut TusdtVaultAlpha) {
    let mut config = default_global_config();
    config.vault_creation_fee = 0;
    vault.set_global_params(config).unwrap();
    // Advance past the 24h timelock, execute, then reset time to 0.
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(
        24 * 60 * 60 * 1_000 + 1,
    );
    vault.execute_global_params_update().unwrap();
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(0);
}

// ---------------------------------------------------------------------------
// Chain-extension mock
// ---------------------------------------------------------------------------

struct MockExtension {
    stake: Option<u64>,
    should_fail: bool,
    /// When set, the caller-forwarded stake pull (func 25) reports a write failure.
    transfer_fails: bool,
}

impl test::ChainExtension for MockExtension {
    fn ext_id(&self) -> u16 {
        0x1000
    }

    fn call(&mut self, func_id: u16, _input: &[u8], output: &mut Vec<u8>) -> u32 {
        if self.should_fail {
            return 1;
        }
        match func_id {
            0 => {
                // get_stake_info_for_hotkey_coldkey_netuid
                let info = self.stake.map(|stake| StakeInfo {
                    hotkey: default_accounts().bob,
                    coldkey: default_accounts().alice,
                    netuid: ink::scale::Compact(1),
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
            15 => {
                // get_alpha_price — 1_000_000_000 = 1 alpha = 1 TAO
                let price: u64 = 1_000_000_000;
                ink::scale::Encode::encode_to(&price, output);
                0
            }
            // 36 = get_stake_availability
            36 => {
                let availability = tusdt_env::StakeAvailability {
                    netuid: 1,
                    total: self.stake.unwrap_or(0),
                    locked: 0,
                    available: self.stake.unwrap_or(0),
                };
                ink::scale::Encode::encode_to(&availability, output);
                0
            }
            // Write ops (2 = remove_stake, 6 = transfer_stake) — no-op success
            2 | 6 => 0,
            // 25 = caller_transfer_stake (caller-forwarded pull) — honours transfer_fails
            25 => {
                if self.transfer_fails {
                    2
                } else {
                    0
                }
            }
            _ => 1,
        }
    }
}

fn register_mock(stake: u64) {
    test::register_chain_extension(MockExtension {
        stake: Some(stake),
        should_fail: false,
        transfer_fails: false,
    });
}

fn register_mock_no_stake() {
    test::register_chain_extension(MockExtension {
        stake: None,
        should_fail: false,
        transfer_fails: false,
    });
}

fn register_mock_transfer_fails(stake: u64) {
    test::register_chain_extension(MockExtension {
        stake: Some(stake),
        should_fail: false,
        transfer_fails: true,
    });
}

// ---------------------------------------------------------------------------
// Existing tests
// ---------------------------------------------------------------------------

#[ink::test]
fn constructor_sets_governance() {
    let (vault, accounts) = setup();
    assert_eq!(vault.governance(), accounts.alice);
}

#[ink::test]
fn paused_by_default() {
    let (vault, _accounts) = setup();
    assert!(!vault.paused());
}

#[ink::test]
fn governance_can_pause_and_unpause() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    vault.pause().unwrap();
    assert!(vault.paused());
    vault.unpause().unwrap();
    assert!(!vault.paused());
}

#[ink::test]
fn non_governance_cannot_pause() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.bob);
    assert!(vault.pause().is_err());
}

#[ink::test]
fn governance_can_manage_approved_netuids() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    assert!(!vault.is_approved_netuid(1));
    vault.set_approved_netuid(1, true).unwrap();
    assert!(vault.is_approved_netuid(1));
    vault.set_approved_netuid(1, false).unwrap();
    assert!(!vault.is_approved_netuid(1));
}

#[ink::test]
fn non_governance_cannot_manage_netuids() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.bob);
    assert!(vault.set_approved_netuid(1, true).is_err());
}

#[ink::test]
fn create_alpha_vault_rejected_for_unapproved_netuid() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    register_mock(100);
    assert_eq!(vault.create_alpha_vault(100, 99), Err(Error::UnapprovedNetuid));
}

#[ink::test]
fn create_alpha_vault_rejects_zero_amount() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    register_mock(100);
    assert_eq!(
        vault.create_alpha_vault(0, 1),
        Err(Error::InsufficientCollateral)
    );
}

#[ink::test]
fn create_alpha_vault_rejected_when_paused() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    vault.pause().unwrap();
    register_mock(100);
    assert_eq!(vault.create_alpha_vault(100, 1), Err(Error::ContractPaused));
}

#[ink::test]
fn create_alpha_vault_pull_failure_leaves_no_state() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    // The caller-forwarded pull reports a write failure (e.g. caller lacks stake).
    register_mock_transfer_fails(0);
    assert_eq!(
        vault.create_alpha_vault(1_000, 1),
        Err(Error::StakeTransferFailed)
    );
    assert_eq!(vault.get_vaults_count(accounts.alice), 0);
    assert_eq!(vault.get_total_vaults_count(), 0);
}

#[ink::test]
fn same_caller_can_open_vaults_on_different_netuids() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    vault.set_approved_netuid(2, true).unwrap();
    register_mock(1_000);
    vault.create_alpha_vault(100, 1).unwrap();
    register_mock(1_000);
    vault.create_alpha_vault(200, 2).unwrap();
    assert_eq!(vault.get_vaults_count(accounts.alice), 2);
}

#[ink::test]
fn per_netuid_params_fallback_to_defaults() {
    let (vault, _accounts) = setup();
    let params = vault.get_contract_params(99);
    assert_eq!(params.collateral_ratio, 15_000);
}

#[ink::test]
fn per_netuid_params_are_isolated() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    let mut config1 = vault.get_contract_params(1);
    config1.collateral_ratio = 20_000;
    vault.set_contract_params(1, config1).unwrap();
    let params2 = vault.get_contract_params(2);
    assert_eq!(params2.collateral_ratio, 15_000);
}

#[ink::test]
fn vault_count_starts_at_zero() {
    let (vault, accounts) = setup();
    assert_eq!(vault.get_vaults_count(accounts.alice), 0);
    assert_eq!(vault.get_total_vaults_count(), 0);
}

// ---------------------------------------------------------------------------
// Auction / Liquidation flow tests
// ---------------------------------------------------------------------------

/// Creates a vault with the given alpha collateral via the atomic pull deposit.
fn create_test_vault(
    vault: &mut TusdtVaultAlpha,
    owner: ink::primitives::AccountId,
    amount: u64,
) -> u32 {
    set_caller(owner);
    register_mock(amount);
    vault.create_alpha_vault(amount, 1).unwrap()
}

#[ink::test]
fn no_liquidation_auction_for_new_vault() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    let vault_id = create_test_vault(&mut vault, accounts.alice, 10_000_000);

    // Fresh vault should have no active liquidation
    assert!(vault.get_liquidation_auction_id(accounts.alice, vault_id).is_none());
    assert_eq!(vault.get_total_vaults_count(), 1);
}

#[ink::test]
fn trigger_liquidation_auction_rejects_duplicate() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    // Manually mark a vault as in-liquidation via the test helper
    vault.set_liquidation_auction_for_test(accounts.alice, 0, 42);

    // Now trigger should fail because an auction already exists
    let result = vault.trigger_liquidation_auction(accounts.alice, 0);
    assert_eq!(result, Err(Error::LiquidationAuctionExists));
}

#[ink::test]
fn settle_liquidation_auction_rejects_when_not_finalized() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    // No auction at all → AuctionNotFound
    let result = vault.settle_liquidation_auction(accounts.bob, 0);
    assert_eq!(result, Err(Error::AuctionNotFound));
}

#[ink::test]
fn vault_in_liquidation_blocks_operations() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    let vault_id = create_test_vault(&mut vault, accounts.alice, 10_000_000);

    // Manually mark as in liquidation
    vault.set_liquidation_auction_for_test(accounts.alice, vault_id, 99);

    // add_alpha_collateral should reject (vault in liquidation)
    set_caller(accounts.alice);
    register_mock(2_000);
    let result = vault.add_alpha_collateral(vault_id, 2_000);
    assert_eq!(result, Err(Error::VaultInLiquidation));
}

#[ink::test]
fn release_alpha_collateral_blocked_during_liquidation() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    let vault_id = create_test_vault(&mut vault, accounts.alice, 10_000_000);

    vault.set_liquidation_auction_for_test(accounts.alice, vault_id, 99);

    set_caller(accounts.alice);
    let result = vault.release_alpha_collateral(vault_id, 100, accounts.alice);
    assert_eq!(result, Err(Error::VaultInLiquidation));
}

#[ink::test]
fn create_alpha_vault_atomic_success() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    register_mock(10_000_000);

    let vault_id = vault.create_alpha_vault(10_000_000, 1).unwrap();
    assert_eq!(vault_id, 0);
    assert_eq!(vault.get_vaults_count(accounts.alice), 1);
    assert_eq!(vault.get_total_vaults_count(), 1);

    let stored = vault.get_vault(accounts.alice, 0).unwrap();
    assert_eq!(stored.collateral_balance, 10_000_000);
    assert_eq!(stored.netuid, 1);
    assert_eq!(stored.owner, accounts.alice);
}

#[ink::test]
fn race_two_users_credited_only_their_own_amount() {
    // Regression for the old aggregate-check race: with the atomic pull, each
    // caller's vault is backed by exactly the amount pulled from THEIR coldkey —
    // cross-attribution is impossible by construction.
    let (mut vault, accounts) = setup_with_approved_netuid();
    register_mock(2_000);

    set_caller(accounts.alice);
    let alice_vault = vault.create_alpha_vault(1_000, 1).unwrap();
    set_caller(accounts.bob);
    let bob_vault = vault.create_alpha_vault(1_000, 1).unwrap();

    assert_eq!(
        vault
            .get_vault(accounts.alice, alice_vault)
            .unwrap()
            .collateral_balance,
        1_000
    );
    assert_eq!(
        vault
            .get_vault(accounts.bob, bob_vault)
            .unwrap()
            .collateral_balance,
        1_000
    );
}

#[ink::test]
fn add_alpha_collateral_pulls_exact_amount() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    let vault_id = create_test_vault(&mut vault, accounts.alice, 10_000_000);

    set_caller(accounts.alice);
    vault.add_alpha_collateral(vault_id, 5_000_000).unwrap();

    let stored = vault.get_vault(accounts.alice, vault_id).unwrap();
    assert_eq!(stored.collateral_balance, 15_000_000);
}

#[ink::test]
fn add_alpha_collateral_rejects_zero_amount() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    let vault_id = create_test_vault(&mut vault, accounts.alice, 10_000_000);

    set_caller(accounts.alice);
    assert_eq!(
        vault.add_alpha_collateral(vault_id, 0),
        Err(Error::InsufficientCollateral)
    );
}

#[ink::test]
fn add_alpha_collateral_pull_failure_leaves_vault_unchanged() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    let vault_id = create_test_vault(&mut vault, accounts.alice, 10_000_000);

    register_mock_transfer_fails(10_000_000);
    set_caller(accounts.alice);
    assert_eq!(
        vault.add_alpha_collateral(vault_id, 5_000_000),
        Err(Error::StakeTransferFailed)
    );

    let stored = vault.get_vault(accounts.alice, vault_id).unwrap();
    assert_eq!(stored.collateral_balance, 10_000_000);
}

// ── claim_excess_alpha ───────────────────────────────────────────────

#[ink::test]
fn governance_can_claim_excess_alpha() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    create_test_vault(&mut vault, accounts.alice, 10_000_000);

    // Mock reports 15M stake — 5M excess over netuid_total_collateral (10M)
    register_mock(15_000_000);
    set_caller(accounts.alice); // governance
    vault.claim_excess_alpha(1).unwrap();
}

#[ink::test]
fn non_governance_cannot_claim_excess_alpha() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    create_test_vault(&mut vault, accounts.alice, 10_000_000);

    register_mock(15_000_000);
    set_caller(accounts.bob); // not governance
    let result = vault.claim_excess_alpha(1);
    assert_eq!(result, Err(Error::NotGovernance));
}

#[ink::test]
fn claim_excess_alpha_noop_when_no_excess() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    create_test_vault(&mut vault, accounts.alice, 10_000_000);

    // Mock still reports 10M — no excess
    register_mock(10_000_000);
    set_caller(accounts.alice);
    vault.claim_excess_alpha(1).unwrap(); // Should succeed as no-op
}

#[ink::test]
fn claim_excess_alpha_noop_when_no_stake_on_netuid() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    create_test_vault(&mut vault, accounts.alice, 10_000_000);

    // Mock returns None for stake info — NoAlphaStakeFound → no-op
    register_mock_no_stake();
    set_caller(accounts.alice);
    vault.claim_excess_alpha(1).unwrap(); // Should succeed as no-op
}

// ---------------------------------------------------------------------------
// Pricing / borrow-limit math
//
// `borrow_token` itself calls the oracle child contract, which the off-chain
// test environment cannot execute; the risk functions take `price: Ratio` as a
// parameter, so the math is exercised directly with constructed prices.
// ---------------------------------------------------------------------------

use tusdt_primitives::Ratio;

fn set_time(timestamp: u64) {
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(timestamp);
}

/// Sets per-netuid collateral/liquidation ratios through the real timelocked
/// path: schedule as governance, advance past the 24h timelock, execute.
fn configure_ratios(
    vault: &mut TusdtVaultAlpha,
    governance: ink::primitives::AccountId,
    netuid: u16,
    cr_bps: u32,
    lr_bps: u32,
) {
    set_caller(governance);
    let mut config = vault.get_contract_params(netuid);
    config.collateral_ratio = cr_bps;
    config.liquidation_ratio = lr_bps;
    vault.set_contract_params(netuid, config).unwrap();
    set_time(24 * 60 * 60 * 1_000 + 1);
    vault.execute_contract_params_update(netuid).unwrap();
}

// ── alpha_price_rao_to_ratio ─────────────────────────────────────────

#[ink::test]
fn alpha_price_to_ratio_realistic() {
    // 377_277 RAO per alpha = 0.000377277 TAO per alpha
    let ratio = TusdtVaultAlpha::alpha_price_rao_to_ratio(377_277).unwrap();
    assert!(!ratio.is_zero());
    // Multiplying back by 1e9 recovers the raw RAO value exactly.
    assert_eq!(ratio.checked_mul_value(1_000_000_000), Some(377_277));
}

#[ink::test]
fn alpha_price_to_ratio_one_to_one() {
    // 1_000_000_000 RAO = 1 alpha = 1 TAO
    let ratio = TusdtVaultAlpha::alpha_price_rao_to_ratio(1_000_000_000).unwrap();
    assert_eq!(ratio, Ratio::one());
}

#[ink::test]
fn alpha_price_to_ratio_zero() {
    let ratio = TusdtVaultAlpha::alpha_price_rao_to_ratio(0).unwrap();
    assert!(ratio.is_zero());
}

#[ink::test]
fn alpha_price_to_ratio_max_u64_no_overflow() {
    let ratio = TusdtVaultAlpha::alpha_price_rao_to_ratio(u64::MAX).unwrap();
    assert!(!ratio.is_zero());
}

// ── collateral_value ─────────────────────────────────────────────────

#[ink::test]
fn collateral_value_basic() {
    // price 0.1 TUSDT per alpha unit × 1000 units = 100
    let price = Ratio::from_basis_points(1_000);
    assert_eq!(TusdtVaultAlpha::collateral_value(price, 1_000).unwrap(), 100);
}

#[ink::test]
fn collateral_value_zero_price() {
    let price = TusdtVaultAlpha::alpha_price_rao_to_ratio(0).unwrap();
    assert_eq!(TusdtVaultAlpha::collateral_value(price, 1_000).unwrap(), 0);
}

// ── max_borrow_allowed ───────────────────────────────────────────────

#[ink::test]
fn max_borrow_allowed_spec_example() {
    // 1000 alpha (1_000_000_000_000 rao), oracle 200 TUSDT/TAO, alpha price
    // 377_277 rao (0.000377277 TAO/alpha), collateral ratio 5× (50_000 bps):
    //   value = 1000 × 200 × 0.000377277 = 75.4554 TUSDT
    //   max borrow = 75.4554 / 5 = 15.09108 TUSDT = 15_091_080_000 units
    let (mut vault, accounts) = setup_with_approved_netuid();
    configure_ratios(&mut vault, accounts.alice, 1, 50_000, 20_000);

    let alpha_to_tao = TusdtVaultAlpha::alpha_price_rao_to_ratio(377_277).unwrap();
    let price = Ratio::from_integer(200).checked_mul(alpha_to_tao).unwrap();

    let value = TusdtVaultAlpha::collateral_value(price, 1_000_000_000_000).unwrap();
    assert_eq!(value, 75_455_400_000);

    let max_borrow = vault
        .max_borrow_allowed(1, price, 1_000_000_000_000)
        .unwrap();
    assert_eq!(max_borrow, 15_091_080_000);
}

#[ink::test]
fn max_borrow_allowed_default_collateral_ratio() {
    // Default CR = 15_000 bps (1.5×): 1000 / 1.5 = 666 (floor)
    let (vault, _accounts) = setup_with_approved_netuid();
    let price = Ratio::one();
    assert_eq!(vault.max_borrow_allowed(1, price, 1_000).unwrap(), 666);
}

#[ink::test]
fn max_borrow_allowed_uses_per_netuid_ratio() {
    // Netuid 2 gets CR 5× while netuid 1 keeps the 1.5× default; the same
    // price and collateral must yield different borrow limits.
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    vault.set_approved_netuid(2, true).unwrap();
    configure_ratios(&mut vault, accounts.alice, 2, 50_000, 20_000);

    let alpha_to_tao = TusdtVaultAlpha::alpha_price_rao_to_ratio(377_277).unwrap();
    let price = Ratio::from_integer(200).checked_mul(alpha_to_tao).unwrap();

    let max_default = vault
        .max_borrow_allowed(1, price, 1_000_000_000_000)
        .unwrap();
    let max_custom = vault
        .max_borrow_allowed(2, price, 1_000_000_000_000)
        .unwrap();
    assert_eq!(max_default, 50_303_600_000); // 75.4554 / 1.5
    assert_eq!(max_custom, 15_091_080_000); // 75.4554 / 5
}

#[ink::test]
fn max_borrow_allowed_rounds_down() {
    // CR 3× on 100 at price 1.0 → 33.33… floors to 33
    let (mut vault, accounts) = setup_with_approved_netuid();
    configure_ratios(&mut vault, accounts.alice, 1, 30_000, 20_000);
    let price = Ratio::one();
    assert_eq!(vault.max_borrow_allowed(1, price, 100).unwrap(), 33);
}

// ── liquidation limit / is_liquidatable ──────────────────────────────

#[ink::test]
fn liquidation_limit_default_ratio() {
    // Default LR = 12_000 bps (1.2×): 1000 / 1.2 = 833 (floor)
    let (vault, _accounts) = setup_with_approved_netuid();
    let price = Ratio::one();
    assert_eq!(vault.liquidation_limit(1, price, 1_000).unwrap(), 833);
}

#[ink::test]
fn liquidation_limit_custom_ratio() {
    // LR 2× (20_000 bps): 1000 / 2 = 500
    let (mut vault, accounts) = setup_with_approved_netuid();
    configure_ratios(&mut vault, accounts.alice, 1, 50_000, 20_000);
    let price = Ratio::one();
    assert_eq!(vault.liquidation_limit(1, price, 1_000).unwrap(), 500);
}

#[ink::test]
fn is_liquidatable_boundary() {
    // Default LR 1.2×, collateral 1000 at price 1.0 → limit 833.
    // Debt 834 is liquidatable; debt at the limit (833) is not.
    let (mut vault, accounts) = setup_with_approved_netuid();
    let vault_id = create_test_vault(&mut vault, accounts.alice, 10_000_000);
    let mut stored = vault.get_vault(accounts.alice, vault_id).unwrap();
    stored.collateral_balance = 1_000;
    let price = Ratio::one();

    stored.borrowed_token_balance = 834;
    assert!(vault.is_liquidatable(price, &stored).unwrap());

    stored.borrowed_token_balance = 833;
    assert!(!vault.is_liquidatable(price, &stored).unwrap());
}

#[ink::test]
fn liquidation_min_bid_includes_fee() {
    // Default liquidation fee 11% (1_100 bps): 1000 debt → min bid 1110
    let (vault, _accounts) = setup_with_approved_netuid();
    assert_eq!(vault.liquidation_min_bid(1, 1_000).unwrap(), 1_110);
}

// ---------------------------------------------------------------------------
// Global (contract-wide) params: defaults, gating, timelock, validation
// ---------------------------------------------------------------------------

fn default_global_config() -> VaultGlobalParamsConfig {
    VaultGlobalParamsConfig {
        transaction_fee: 30,
        auction_duration_ms: 3_600_000,
        max_oracle_age_ms: 1_800_000,
        vault_creation_fee: 0,
    }
}

#[ink::test]
fn global_params_defaults() {
    let (vault, _accounts) = setup();
    let params = vault.get_global_params();
    assert_eq!(params.transaction_fee, 30);
    assert_eq!(params.auction_duration_ms, 3_600_000);
    assert_eq!(params.max_oracle_age_ms, 1_800_000);
}

#[ink::test]
fn set_global_params_rejects_non_governance() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.bob);
    assert_eq!(
        vault.set_global_params(default_global_config()),
        Err(Error::NotGovernance)
    );
}

#[ink::test]
fn execute_global_params_before_timelock_fails() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    let mut config = default_global_config();
    config.transaction_fee = 50;
    vault.set_global_params(config).unwrap();

    assert_eq!(
        vault.execute_global_params_update(),
        Err(Error::ContractParamsUpdateTimelockActive)
    );
}

#[ink::test]
fn execute_global_params_after_timelock_applies() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    let mut config = default_global_config();
    config.transaction_fee = 50;
    config.auction_duration_ms = 7_200_000;
    config.max_oracle_age_ms = 900_000;
    vault.set_global_params(config).unwrap();

    set_time(24 * 60 * 60 * 1_000 + 1);
    // Execute is permissionless.
    set_caller(accounts.bob);
    vault.execute_global_params_update().unwrap();

    let params = vault.get_global_params();
    assert_eq!(params.transaction_fee, 50);
    assert_eq!(params.auction_duration_ms, 7_200_000);
    assert_eq!(params.max_oracle_age_ms, 900_000);
}

#[ink::test]
fn execute_global_params_without_pending_fails() {
    let (mut vault, _accounts) = setup();
    assert_eq!(
        vault.execute_global_params_update(),
        Err(Error::NoPendingContractParamsUpdate)
    );
}

#[ink::test]
fn cancel_global_params_update_rejects_non_governance() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    vault.set_global_params(default_global_config()).unwrap();

    set_caller(accounts.bob);
    assert_eq!(
        vault.cancel_global_params_update(),
        Err(Error::NotGovernance)
    );
}

#[ink::test]
fn cancel_global_params_update_works() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    vault.set_global_params(default_global_config()).unwrap();
    vault.cancel_global_params_update().unwrap();

    // Nothing pending anymore — execute fails even after the timelock.
    set_time(24 * 60 * 60 * 1_000 + 1);
    assert_eq!(
        vault.execute_global_params_update(),
        Err(Error::NoPendingContractParamsUpdate)
    );
}

#[ink::test]
fn cancel_global_params_update_without_pending_fails() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    assert_eq!(
        vault.cancel_global_params_update(),
        Err(Error::NoPendingContractParamsUpdate)
    );
}

#[ink::test]
fn set_global_params_rejects_fee_above_100_percent() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    let mut config = default_global_config();
    config.transaction_fee = 10_001;
    assert_eq!(vault.set_global_params(config), Err(Error::InvalidRatio));
}

#[ink::test]
fn set_global_params_rejects_short_auction_duration() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    let mut config = default_global_config();
    config.auction_duration_ms = 59_999;
    assert_eq!(
        vault.set_global_params(config),
        Err(Error::InvalidAuctionDuration)
    );
}

#[ink::test]
fn set_global_params_rejects_long_auction_duration() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    let mut config = default_global_config();
    config.auction_duration_ms = 7 * 24 * 60 * 60 * 1_000 + 1;
    assert_eq!(
        vault.set_global_params(config),
        Err(Error::InvalidAuctionDuration)
    );
}

#[ink::test]
fn set_global_params_rejects_zero_oracle_age() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    let mut config = default_global_config();
    config.max_oracle_age_ms = 0;
    assert_eq!(
        vault.set_global_params(config),
        Err(Error::InvalidOracleMaxAge)
    );
}

#[ink::test]
fn transaction_fee_uses_global_params() {
    let (mut vault, accounts) = setup();

    // Default fee 30 bps: 0.3% of 1_000_000 = 3_000.
    assert_eq!(vault.calculate_transaction_fee(1_000_000).unwrap(), 3_000);

    // Raise the global fee to 100 bps (1%) through the timelocked path.
    set_caller(accounts.alice);
    let mut config = default_global_config();
    config.transaction_fee = 100;
    vault.set_global_params(config).unwrap();
    set_time(24 * 60 * 60 * 1_000 + 1);
    vault.execute_global_params_update().unwrap();

    assert_eq!(vault.calculate_transaction_fee(1_000_000).unwrap(), 10_000);
}
