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
    (vault, accounts)
}

// ---------------------------------------------------------------------------
// Chain-extension mock
// ---------------------------------------------------------------------------

struct MockExtension {
    stake: Option<u64>,
    should_fail: bool,
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
            _ => 1,
        }
    }
}

fn register_mock(stake: u64) {
    test::register_chain_extension(MockExtension {
        stake: Some(stake),
        should_fail: false,
    });
}

fn register_mock_no_stake() {
    test::register_chain_extension(MockExtension {
        stake: None,
        should_fail: false,
    });
}

// ---------------------------------------------------------------------------
// Existing tests
// ---------------------------------------------------------------------------

#[ink::test]
fn constructor_sets_governance() {
    let (vault, accounts) = setup();
    assert_eq!(vault.governance(), accounts.alice);
    assert_eq!(vault.alpha_price_netuid(), 0);
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
fn deposit_alpha_rejected_for_unapproved_netuid() {
    let (mut vault, accounts) = setup();
    set_caller(accounts.alice);
    assert!(vault.deposit_alpha(100, 99).is_err());
}

#[ink::test]
fn deposit_alpha_registers_intent() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    vault.deposit_alpha(100, 1).unwrap();
}

#[ink::test]
fn create_alpha_vault_fails_without_deposit() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.bob);
    assert!(vault.create_alpha_vault(1).is_err());
}

#[ink::test]
fn create_alpha_vault_fails_for_wrong_depositor() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    vault.deposit_alpha(100, 1).unwrap();
    set_caller(accounts.bob);
    assert!(vault.create_alpha_vault(1).is_err());
}

#[ink::test]
fn duplicate_deposit_rejected() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    vault.deposit_alpha(100, 1).unwrap();
    assert!(vault.deposit_alpha(200, 1).is_err());
}

#[ink::test]
fn same_caller_different_netuids_allowed() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    vault.set_approved_netuid(2, true).unwrap();
    vault.deposit_alpha(100, 1).unwrap();
    vault.deposit_alpha(200, 2).unwrap();
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

/// Creates a vault with 1000 alpha collateral via the two-step deposit flow.
fn create_test_vault(
    vault: &mut TusdtVaultAlpha,
    owner: ink::primitives::AccountId,
    amount: u64,
) -> u32 {
    set_caller(owner);
    vault.deposit_alpha(amount, 1).unwrap();
    register_mock(amount);
    let vault_id = vault.create_alpha_vault(1).unwrap();
    vault_id
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
    let result = vault.add_alpha_collateral(vault_id);
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
fn create_alpha_vault_checks_stake_availability() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    vault.deposit_alpha(10_000_000, 1).unwrap();

    // Mock reports only 5M staked — less than pending deposit + caps
    register_mock(5_000_000);
    let result = vault.create_alpha_vault(1);
    assert_eq!(result, Err(Error::InsufficientCollateral));
}

#[ink::test]
fn create_alpha_vault_succeeds_with_sufficient_stake() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    set_caller(accounts.alice);
    vault.deposit_alpha(10_000_000, 1).unwrap();

    register_mock(10_000_000);
    let vault_id = vault.create_alpha_vault(1).unwrap();
    assert_eq!(vault_id, 0);
    assert_eq!(vault.get_vaults_count(accounts.alice), 1);
    assert_eq!(vault.get_total_vaults_count(), 1);

    let stored = vault.get_vault(accounts.alice, 0).unwrap();
    assert_eq!(stored.collateral_balance, 10_000_000);
    assert_eq!(stored.netuid, 1);
    assert_eq!(stored.owner, accounts.alice);
}

#[ink::test]
fn add_alpha_collateral_syncs_stake_delta() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    let vault_id = create_test_vault(&mut vault, accounts.alice, 10_000_000);

    // Simulate more stake transferred externally — mock now reports 15M
    register_mock(15_000_000);
    set_caller(accounts.alice);
    vault.add_alpha_collateral(vault_id).unwrap();

    let stored = vault.get_vault(accounts.alice, vault_id).unwrap();
    assert_eq!(stored.collateral_balance, 15_000_000);
}

#[ink::test]
fn add_alpha_collateral_rejects_if_no_new_stake() {
    let (mut vault, accounts) = setup_with_approved_netuid();
    let vault_id = create_test_vault(&mut vault, accounts.alice, 10_000_000);

    // Mock still reports 10M — no new stake
    register_mock(10_000_000);
    set_caller(accounts.alice);
    let result = vault.add_alpha_collateral(vault_id);
    assert_eq!(result, Err(Error::InsufficientCollateral));
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
