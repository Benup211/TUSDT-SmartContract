use super::vault::*;
use tusdt_oracle::PriceData;
use tusdt_primitives::{Ratio, MILLISECONDS_PER_HOUR};

const TEST_MIN_VAULT_COLLATERAL: u64 = 5_000_000;
const TEST_MAX_VAULT_COLLATERAL: u64 = 1_000_000_000;
const TEST_MAX_TOTAL_COLLATERAL: u64 = 21_000_000_000;

fn opening_collateral(extra: u64) -> u64 {
    TEST_MIN_VAULT_COLLATERAL + extra
}

fn set_caller(caller: ink::primitives::AccountId) {
    let callee = ink::env::account_id::<tusdt_env::CustomEnvironment>();
    ink::env::test::set_callee::<tusdt_env::CustomEnvironment>(callee);
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(caller);
}

fn set_time(timestamp: u64) {
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(timestamp);
}

fn set_transferred_value(value: u64) {
    ink::env::test::set_value_transferred::<ink::env::DefaultEnvironment>(u128::from(value));
}

fn account_balance(account: ink::primitives::AccountId) -> u128 {
    ink::env::test::get_account_balance::<ink::env::DefaultEnvironment>(account).unwrap_or(0)
}

fn fund_caller_for_transfer(value: u64) {
    let caller = ink::env::caller::<tusdt_env::CustomEnvironment>();
    let current_balance = account_balance(caller);
    let required_balance = current_balance.saturating_add(u128::from(value));
    ink::env::test::set_account_balance::<ink::env::DefaultEnvironment>(caller, required_balance);
}

fn transfer_in(value: u64) {
    fund_caller_for_transfer(value);
    ink::env::test::transfer_in::<ink::env::DefaultEnvironment>(u128::from(value));
}

fn create_vault_with_collateral(
    contract: &mut TusdtVault,
    owner: ink::primitives::AccountId,
    collateral: u64,
) -> u32 {
    set_caller(owner);
    transfer_in(collateral);
    contract
        .create_vault()
        .expect("create_vault should succeed in tests")
}

fn test_contract_params() -> VaultContractParamsConfig {
    VaultContractParamsConfig {
        collateral_ratio: 20_000,
        liquidation_ratio: 13_000,
        interest_rate: 700,
        liquidation_fee: 200,
        borrow_cap: 1_000_000,
        min_vault_collateral: TEST_MIN_VAULT_COLLATERAL,
        max_vault_collateral: TEST_MAX_VAULT_COLLATERAL,
        max_total_collateral: TEST_MAX_TOTAL_COLLATERAL,
        transaction_fee: 3,
        auction_duration_ms: 120_000,
        max_oracle_age_ms: 600_000,
    }
}

#[ink::test]
fn create_vault_tracks_ids_balances_and_counts() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);
    let alice_collateral_0 = opening_collateral(500);
    let alice_collateral_1 = opening_collateral(700);
    let bob_collateral_0 = opening_collateral(300);

    set_time(10);
    let alice_vault_0 =
        create_vault_with_collateral(&mut vault, accounts.alice, alice_collateral_0);
    set_time(20);
    let alice_vault_1 =
        create_vault_with_collateral(&mut vault, accounts.alice, alice_collateral_1);
    set_time(30);
    let bob_vault_0 = create_vault_with_collateral(&mut vault, accounts.bob, bob_collateral_0);

    assert_eq!(alice_vault_0, 0);
    assert_eq!(alice_vault_1, 1);
    assert_eq!(bob_vault_0, 0);
    assert_eq!(vault.get_vaults_count(accounts.alice), 2);
    assert_eq!(vault.get_vaults_count(accounts.bob), 1);
    assert_eq!(vault.get_total_vaults_count(), 3);
    assert_eq!(
        vault.get_total_collateral_balance(),
        alice_collateral_0 + alice_collateral_1 + bob_collateral_0
    );
    assert_eq!(vault.get_total_debt(accounts.alice), 0);
    assert_eq!(vault.get_total_debt(accounts.bob), 0);

    let alice_v0 = vault
        .get_vault(accounts.alice, 0)
        .expect("alice vault 0 should exist");
    assert_eq!(alice_v0.owner, accounts.alice);
    assert_eq!(alice_v0.collateral_balance, alice_collateral_0);
    assert_eq!(alice_v0.borrowed_token_balance, 0);
    assert_eq!(alice_v0.debt_balance, 0);
    assert_eq!(alice_v0.total_interest_accrued, 0);
    assert_eq!(alice_v0.created_at, 10);
    assert_eq!(alice_v0.last_interest_accrued_at, 10);

    let bob_v0 = vault
        .get_vault(accounts.bob, 0)
        .expect("bob vault 0 should exist");
    assert_eq!(bob_v0.collateral_balance, bob_collateral_0);
    assert_eq!(bob_v0.created_at, 30);
}

#[ink::test]
fn add_collateral_updates_vault_and_total_collateral() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);
    let initial_collateral = opening_collateral(400);

    create_vault_with_collateral(&mut vault, accounts.alice, initial_collateral);

    set_caller(accounts.alice);
    transfer_in(250);
    assert_eq!(vault.add_collateral(0), Ok(()));

    let updated = vault
        .get_vault(accounts.alice, 0)
        .expect("vault should exist after add_collateral");
    assert_eq!(updated.collateral_balance, initial_collateral + 250);
    assert_eq!(
        vault.get_total_collateral_balance(),
        initial_collateral + 250
    );
}

#[ink::test]
fn create_vault_rejects_collateral_below_minimum() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    set_caller(accounts.alice);
    transfer_in(TEST_MIN_VAULT_COLLATERAL - 1);
    assert_eq!(vault.create_vault(), Err(Error::InsufficientCollateral));
    assert_eq!(vault.get_vaults_count(accounts.alice), 0);
    assert_eq!(vault.get_total_vaults_count(), 0);
    assert_eq!(vault.get_total_collateral_balance(), 0);
}

#[ink::test]
fn add_collateral_fails_for_missing_or_liquidating_vault() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    set_caller(accounts.bob);
    set_transferred_value(10);
    assert_eq!(vault.add_collateral(0), Err(Error::VaultNotFound));

    create_vault_with_collateral(&mut vault, accounts.alice, opening_collateral(200));
    vault.set_liquidation_auction_for_test(accounts.alice, 0, 42);

    set_caller(accounts.alice);
    set_transferred_value(10);
    assert_eq!(vault.add_collateral(0), Err(Error::VaultInLiquidation));
}

#[ink::test]
fn release_collateral_checks_balance_before_oracle_path() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);
    let collateral = opening_collateral(150);

    create_vault_with_collateral(&mut vault, accounts.alice, collateral);

    set_caller(accounts.alice);
    assert_eq!(
        vault.release_collateral(0, collateral + 1),
        Err(Error::InsufficientCollateral)
    );
}

#[ink::test]
fn set_contract_params_enforces_governance_and_validation() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);
    let original = vault.get_contract_params();

    let valid = test_contract_params();

    set_caller(accounts.bob);
    assert_eq!(vault.set_contract_params(valid), Err(Error::NotGovernance));

    set_caller(accounts.alice);
    assert_eq!(
        vault.set_contract_params(VaultContractParamsConfig {
            collateral_ratio: 9_900,
            ..valid
        }),
        Err(Error::InvalidRatio)
    );
    assert_eq!(
        vault.set_contract_params(VaultContractParamsConfig {
            auction_duration_ms: 59_999,
            ..valid
        }),
        Err(Error::InvalidAuctionDuration)
    );
    assert_eq!(
        vault.set_contract_params(VaultContractParamsConfig {
            auction_duration_ms: 604_800_001,
            ..valid
        }),
        Err(Error::InvalidAuctionDuration)
    );
    assert_eq!(
        vault.set_contract_params(VaultContractParamsConfig {
            max_oracle_age_ms: 0,
            ..valid
        }),
        Err(Error::InvalidOracleMaxAge)
    );
    assert_eq!(
        vault.set_contract_params(VaultContractParamsConfig {
            transaction_fee: 10_001,
            ..valid
        }),
        Err(Error::InvalidRatio)
    );
    assert_eq!(
        vault.set_contract_params(VaultContractParamsConfig {
            collateral_ratio: 13_000,
            ..valid
        }),
        Err(Error::InvalidRatio)
    );
    assert_eq!(
        vault.set_contract_params(VaultContractParamsConfig {
            collateral_ratio: 12_000,
            ..valid
        }),
        Err(Error::InvalidRatio)
    );

    set_time(0);
    assert_eq!(vault.set_contract_params(valid), Ok(()));
    let params = vault.get_contract_params();
    assert_eq!(params.collateral_ratio, original.collateral_ratio);
    assert_eq!(params.liquidation_ratio, original.liquidation_ratio);
    assert_eq!(params.interest_rate, original.interest_rate);
    assert_eq!(params.liquidation_fee, original.liquidation_fee);
    assert_eq!(params.borrow_cap, original.borrow_cap);
    assert_eq!(params.transaction_fee, original.transaction_fee);
    assert_eq!(params.auction_duration_ms, original.auction_duration_ms);
    assert_eq!(params.max_oracle_age_ms, original.max_oracle_age_ms);

    let pending = vault
        .get_pending_contract_params_update()
        .expect("pending params update should be stored");
    assert_eq!(pending.params.collateral_ratio, 20_000);
    assert_eq!(pending.params.liquidation_ratio, 13_000);
    assert_eq!(pending.params.interest_rate, 700);
    assert_eq!(pending.params.liquidation_fee, 200);
    assert_eq!(pending.params.borrow_cap, 1_000_000);
    assert_eq!(pending.params.transaction_fee, 3);
    assert_eq!(pending.params.auction_duration_ms, 120_000);
    assert_eq!(pending.params.max_oracle_age_ms, 600_000);

    assert_eq!(
        vault.execute_contract_params_update(),
        Err(Error::ContractParamsUpdateTimelockActive)
    );

    set_time(CONTRACT_PARAMS_TIMELOCK_MS);
    set_caller(accounts.bob);
    assert_eq!(vault.execute_contract_params_update(), Ok(()));
    assert!(vault.get_pending_contract_params_update().is_none());

    let params = vault.get_contract_params();
    assert_eq!(params.collateral_ratio, 20_000);
    assert_eq!(params.liquidation_ratio, 13_000);
    assert_eq!(params.interest_rate, 700);
    assert_eq!(params.liquidation_fee, 200);
    assert_eq!(params.borrow_cap, 1_000_000);
    assert_eq!(params.transaction_fee, 3);
    assert_eq!(params.auction_duration_ms, 120_000);
    assert_eq!(params.max_oracle_age_ms, 600_000);
}

#[ink::test]
fn governance_can_be_updated_by_current_governance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);
    let valid = test_contract_params();

    set_caller(accounts.bob);
    assert_eq!(
        vault.update_governance(accounts.bob),
        Err(Error::NotGovernance)
    );

    set_caller(accounts.alice);
    assert_eq!(vault.governance(), accounts.alice);
    assert_eq!(vault.update_governance(accounts.bob), Ok(()));
    assert_eq!(vault.governance(), accounts.bob);

    assert_eq!(vault.set_contract_params(valid), Err(Error::NotGovernance));

    set_caller(accounts.bob);
    assert_eq!(vault.set_contract_params(valid), Ok(()));
    assert_eq!(
        vault.execute_contract_params_update(),
        Err(Error::ContractParamsUpdateTimelockActive)
    );

    set_time(CONTRACT_PARAMS_TIMELOCK_MS);
    assert_eq!(vault.execute_contract_params_update(), Ok(()));
    assert_eq!(vault.get_contract_params().collateral_ratio, 20_000);
}

#[ink::test]
fn treasury_can_be_updated_by_governance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    assert_eq!(vault.treasury(), accounts.alice);

    set_caller(accounts.bob);
    assert_eq!(
        vault.update_treasury(accounts.charlie),
        Err(Error::NotGovernance)
    );
    assert_eq!(vault.treasury(), accounts.alice);

    set_caller(accounts.alice);
    assert_eq!(vault.update_treasury(accounts.charlie), Ok(()));
    assert_eq!(vault.treasury(), accounts.charlie);

    assert_eq!(vault.update_governance(accounts.bob), Ok(()));
    assert_eq!(vault.governance(), accounts.bob);
    assert_eq!(vault.treasury(), accounts.charlie);
}

#[ink::test]
fn governance_can_cancel_pending_contract_params_update() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);
    let valid = test_contract_params();

    set_caller(accounts.alice);
    set_time(0);
    assert_eq!(vault.set_contract_params(valid), Ok(()));
    assert!(vault.get_pending_contract_params_update().is_some());

    set_caller(accounts.bob);
    assert_eq!(
        vault.cancel_contract_params_update(),
        Err(Error::NotGovernance)
    );

    set_caller(accounts.alice);
    assert_eq!(vault.cancel_contract_params_update(), Ok(()));
    assert!(vault.get_pending_contract_params_update().is_none());
    assert_eq!(
        vault.execute_contract_params_update(),
        Err(Error::NoPendingContractParamsUpdate)
    );
}

#[ink::test]
fn governance_can_pause_and_unpause_contract() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    assert!(!vault.paused());

    set_caller(accounts.bob);
    assert_eq!(vault.pause(), Err(Error::NotGovernanceOrPlatform));
    assert!(!vault.paused());

    set_caller(accounts.alice);
    assert_eq!(vault.pause(), Ok(()));
    assert!(vault.paused());

    assert_eq!(vault.unpause(), Ok(()));
    assert!(!vault.paused());
}

#[ink::test]
fn platform_can_pause_but_not_unpause() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    set_caller(accounts.alice);
    assert_eq!(vault.update_platform(accounts.bob), Ok(()));

    set_caller(accounts.bob);
    assert_eq!(vault.pause(), Ok(()));
    assert!(vault.paused());

    assert_eq!(vault.unpause(), Err(Error::NotGovernance));
    assert!(vault.paused());

    set_caller(accounts.alice);
    assert_eq!(vault.unpause(), Ok(()));
    assert!(!vault.paused());
}

#[ink::test]
fn paused_contract_rejects_mutations() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    let vault_id =
        create_vault_with_collateral(&mut vault, accounts.alice, opening_collateral(400));

    set_caller(accounts.alice);
    assert_eq!(vault.pause(), Ok(()));

    transfer_in(100);
    assert_eq!(vault.create_vault(), Err(Error::ContractPaused));

    transfer_in(50);
    assert_eq!(vault.add_collateral(vault_id), Err(Error::ContractPaused));

    assert_eq!(
        vault.release_collateral(vault_id, 10),
        Err(Error::ContractPaused)
    );
    assert_eq!(
        vault.accrue_interest(accounts.alice, vault_id),
        Err(Error::ContractPaused)
    );
    assert_eq!(
        vault.trigger_liquidation_auction(accounts.alice, vault_id),
        Err(Error::ContractPaused)
    );
}

#[ink::test]
fn paused_contract_still_allows_liquidation_settlement_path() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    let vault_id =
        create_vault_with_collateral(&mut vault, accounts.alice, opening_collateral(400));

    set_caller(accounts.alice);
    assert_eq!(vault.pause(), Ok(()));

    assert_eq!(
        vault.settle_liquidation_auction(accounts.alice, vault_id),
        Err(Error::AuctionNotFound)
    );
}

#[ink::test]
fn price_validation_and_collateral_math_helpers_work() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let vault = TusdtVault::new_for_test(accounts.alice);
    let price = Ratio::from_integer(3);

    assert_eq!(TusdtVault::collateral_value(price, 100), Ok(300));
    assert_eq!(vault.max_borrow_allowed(price, 100), Ok(200));
    assert_eq!(vault.liquidation_limit(price, 100), Ok(250));
    assert_eq!(vault.liquidation_min_bid(300), Ok(333));
}

#[ink::test]
fn oracle_price_validation_rejects_missing_and_stale_data() {
    let price = Ratio::from_integer(3);

    assert_eq!(
        TusdtVault::validate_price_data(None, 10, 100),
        Err(Error::OraclePriceUnavailable)
    );
    assert_eq!(
        TusdtVault::validate_price_data(
            Some(PriceData {
                round_id: 0,
                price,
                median_price: price,
                reporter_count: 3,
                committed_at: 0,
                was_overridden: false,
            }),
            3_600_001,
            3_600_000,
        ),
        Err(Error::OraclePriceStale)
    );
    assert_eq!(
        TusdtVault::validate_price_data(
            Some(PriceData {
                round_id: 1,
                price,
                median_price: price,
                reporter_count: 3,
                committed_at: 10,
                was_overridden: false,
            }),
            20,
            30,
        ),
        Ok(PriceData {
            round_id: 1,
            price,
            median_price: price,
            reporter_count: 3,
            committed_at: 10,
            was_overridden: false,
        })
    );
}

#[ink::test]
fn pagination_for_owner_and_global_vaults_works() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    for _ in 0..12 {
        create_vault_with_collateral(&mut vault, accounts.alice, TEST_MIN_VAULT_COLLATERAL);
    }

    let owner_page_0 = vault
        .get_vaults(accounts.alice, 0)
        .expect("owner page 0 should exist");
    assert_eq!(owner_page_0.len(), 10);
    assert_eq!(owner_page_0[0].id, 0);
    assert_eq!(owner_page_0[9].id, 9);

    let owner_page_1 = vault
        .get_vaults(accounts.alice, 1)
        .expect("owner page 1 should exist");
    assert_eq!(owner_page_1.len(), 2);
    assert_eq!(owner_page_1[0].id, 10);
    assert_eq!(owner_page_1[1].id, 11);

    assert!(matches!(
        vault.get_vaults(accounts.alice, 2),
        Ok(vaults) if vaults.is_empty()
    ));

    let all_page_0 = vault.get_all_vaults(0).expect("global page 0 should exist");
    assert_eq!(all_page_0.len(), 10);
    let all_page_1 = vault.get_all_vaults(1).expect("global page 1 should exist");
    assert_eq!(all_page_1.len(), 2);

    assert!(matches!(
        vault.get_all_vaults(2),
        Ok(vaults) if vaults.is_empty()
    ));
}

#[ink::test]
fn interest_accrues_at_beginning_of_hour() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let vault_contract = TusdtVault::new_for_test(accounts.alice);
    let mut vault = Vault {
        id: 0,
        owner: accounts.alice,
        collateral_balance: 1_000,
        borrowed_token_balance: 1_000_000,
        debt_balance: 1_000_000,
        total_interest_accrued: 0,
        created_at: 0,
        last_interest_accrued_at: 0,
    };

    set_time(MILLISECONDS_PER_HOUR / 2);
    assert_eq!(vault_contract.accrue_interest_for_vault(&mut vault), Ok(()));
    assert_eq!(vault.borrowed_token_balance, 1_000_000);
    assert!(vault.debt_balance > 1_000_000);
    assert_eq!(vault.total_interest_accrued, vault.debt_balance - 1_000_000);
    assert_eq!(vault.last_interest_accrued_at, MILLISECONDS_PER_HOUR);

    set_time(MILLISECONDS_PER_HOUR);
    assert_eq!(vault_contract.accrue_interest_for_vault(&mut vault), Ok(()));
    assert_eq!(vault.last_interest_accrued_at, MILLISECONDS_PER_HOUR);
}

#[ink::test]
fn zero_amount_borrow_charges_current_hour() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault_contract = TusdtVault::new_for_test(accounts.alice);

    set_time(0);
    let vault_id = create_vault_with_collateral(
        &mut vault_contract,
        accounts.alice,
        opening_collateral(1_000),
    );

    let mut stored_vault = vault_contract
        .get_vault(accounts.alice, vault_id)
        .expect("vault should exist");
    stored_vault.borrowed_token_balance = 1_000_000;
    stored_vault.debt_balance = 1_000_000;
    stored_vault.last_interest_accrued_at = 0;
    assert_eq!(
        vault_contract.save_vault(accounts.alice, vault_id, &stored_vault),
        Ok(())
    );

    set_caller(accounts.alice);
    set_transferred_value(0);
    set_time(MILLISECONDS_PER_HOUR / 2);
    assert_eq!(vault_contract.borrow_token(vault_id, 0), Ok(()));

    let after_zero_borrow = vault_contract
        .get_vault(accounts.alice, vault_id)
        .expect("vault should still exist");
    assert_eq!(after_zero_borrow.borrowed_token_balance, 1_000_000);
    assert!(after_zero_borrow.debt_balance > 1_000_000);
    assert_eq!(
        after_zero_borrow.last_interest_accrued_at,
        MILLISECONDS_PER_HOUR
    );

    set_time(MILLISECONDS_PER_HOUR + 1);
    let accrued_balance = vault_contract
        .accrue_interest(accounts.alice, vault_id)
        .expect("interest accrual should succeed");
    assert!(accrued_balance > after_zero_borrow.debt_balance);

    let accrued_vault = vault_contract
        .get_vault(accounts.alice, vault_id)
        .expect("vault should still exist");
    assert_eq!(
        accrued_vault.last_interest_accrued_at,
        2 * MILLISECONDS_PER_HOUR
    );
}

#[ink::test]
fn transaction_fee_helpers_use_percentage_math() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    set_caller(accounts.alice);
    set_time(0);
    assert_eq!(vault.set_contract_params(test_contract_params()), Ok(()));
    set_time(CONTRACT_PARAMS_TIMELOCK_MS);
    assert_eq!(vault.execute_contract_params_update(), Ok(()));

    assert_eq!(vault.calculate_transaction_fee(10_000), Ok(3));
    assert_eq!(vault.calculate_transaction_fee(3), Ok(0));

    assert_eq!(vault.transfer_transaction_fee_to_treasury(0), Ok(()));
}

#[ink::test]
fn vault_params_support_fractional_percentages() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    let fractional = VaultContractParamsConfig {
        collateral_ratio: 15_050,
        liquidation_ratio: 12_025,
        interest_rate: 75,
        liquidation_fee: 25,
        borrow_cap: 1_000_000,
        min_vault_collateral: TEST_MIN_VAULT_COLLATERAL,
        max_vault_collateral: TEST_MAX_VAULT_COLLATERAL,
        max_total_collateral: TEST_MAX_TOTAL_COLLATERAL,
        transaction_fee: 50,
        auction_duration_ms: 120_000,
        max_oracle_age_ms: 600_000,
    };

    set_caller(accounts.alice);
    set_time(0);
    assert_eq!(vault.set_contract_params(fractional), Ok(()));
    set_time(CONTRACT_PARAMS_TIMELOCK_MS);
    assert_eq!(vault.execute_contract_params_update(), Ok(()));

    let stored = vault.get_contract_params();
    assert_eq!(stored.collateral_ratio, 15_050);
    assert_eq!(stored.liquidation_ratio, 12_025);
    assert_eq!(stored.interest_rate, 75);
    assert_eq!(stored.liquidation_fee, 25);
    assert_eq!(stored.transaction_fee, 50);
    assert_eq!(vault.calculate_transaction_fee(1_000), Ok(5));
}

#[ink::test]
fn zero_amount_repay_skips_transaction_fee_check() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault_contract = TusdtVault::new_for_test(accounts.alice);

    set_time(0);
    let vault_id = create_vault_with_collateral(
        &mut vault_contract,
        accounts.alice,
        opening_collateral(1_000),
    );

    let mut stored_vault = vault_contract
        .get_vault(accounts.alice, vault_id)
        .expect("vault should exist");
    stored_vault.borrowed_token_balance = 1_000_000;
    stored_vault.debt_balance = 1_000_000;
    stored_vault.last_interest_accrued_at = 0;
    assert_eq!(
        vault_contract.save_vault(accounts.alice, vault_id, &stored_vault),
        Ok(())
    );

    set_caller(accounts.alice);
    set_transferred_value(0);
    assert_eq!(vault_contract.repay_token(vault_id, 0), Ok(()));
}

#[ink::test]
fn new_borrow_reweights_timestamp_toward_now() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let vault_contract = TusdtVault::new_for_test(accounts.alice);
    let mut vault = Vault {
        id: 0,
        owner: accounts.alice,
        collateral_balance: 1_000,
        borrowed_token_balance: 1_000_000,
        debt_balance: 1_000_000,
        total_interest_accrued: 0,
        created_at: 0,
        last_interest_accrued_at: MILLISECONDS_PER_HOUR,
    };

    set_time(MILLISECONDS_PER_HOUR / 2);
    assert_eq!(
        vault_contract.adjust_last_interest_accrued_at_for_new_borrow(&mut vault, 1_000_000),
        Ok(())
    );
    assert_eq!(
        vault.last_interest_accrued_at,
        (3 * MILLISECONDS_PER_HOUR) / 4
    );

    vault.borrowed_token_balance = 2_000_000;
    vault.debt_balance = 2_000_000;
    set_time((3 * MILLISECONDS_PER_HOUR) / 4);
    assert_eq!(vault_contract.accrue_interest_for_vault(&mut vault), Ok(()));
    assert_eq!(vault.borrowed_token_balance, 2_000_000);
    assert_eq!(vault.debt_balance, 2_000_000);
    assert_eq!(
        vault.last_interest_accrued_at,
        (3 * MILLISECONDS_PER_HOUR) / 4
    );

    set_time((3 * MILLISECONDS_PER_HOUR) / 4 + 1);
    assert_eq!(vault_contract.accrue_interest_for_vault(&mut vault), Ok(()));
    assert_eq!(vault.borrowed_token_balance, 2_000_000);
    assert!(vault.debt_balance > 2_000_000);
    assert_eq!(
        vault.last_interest_accrued_at,
        (7 * MILLISECONDS_PER_HOUR) / 4
    );
}

#[ink::test]
fn interest_uses_hourly_discrete_compounding() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault_contract = TusdtVault::new_for_test(accounts.alice);
    let mut vault = Vault {
        id: 0,
        owner: accounts.alice,
        collateral_balance: 1_000,
        borrowed_token_balance: 100_000,
        debt_balance: 100_000,
        total_interest_accrued: 0,
        created_at: 0,
        last_interest_accrued_at: 0,
    };

    set_caller(accounts.alice);
    set_time(0);
    assert_eq!(
        vault_contract.set_contract_params(VaultContractParamsConfig {
            interest_rate: 1_000,
            ..test_contract_params()
        }),
        Ok(())
    );
    set_time(CONTRACT_PARAMS_TIMELOCK_MS);
    assert_eq!(vault_contract.execute_contract_params_update(), Ok(()));

    set_time(30 * 24 * MILLISECONDS_PER_HOUR);
    assert_eq!(vault_contract.accrue_interest_for_vault(&mut vault), Ok(()));
    assert_eq!(vault.borrowed_token_balance, 100_000);
    assert_eq!(vault.debt_balance, 100_826);
    assert_eq!(vault.total_interest_accrued, 826);
    assert_eq!(
        vault.last_interest_accrued_at,
        (30 * 24 + 1) * MILLISECONDS_PER_HOUR
    );
}

#[ink::test]
fn interest_accrual_saturates_at_balance_max_instead_of_reverting() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let vault_contract = TusdtVault::new_for_test(accounts.alice);
    let mut vault = Vault {
        id: 0,
        owner: accounts.alice,
        collateral_balance: 1_000,
        borrowed_token_balance: u64::MAX - 1,
        debt_balance: u64::MAX - 1,
        total_interest_accrued: 0,
        created_at: 0,
        last_interest_accrued_at: 0,
    };

    set_time(1);
    assert_eq!(vault_contract.accrue_interest_for_vault(&mut vault), Ok(()));
    assert_eq!(vault.borrowed_token_balance, u64::MAX - 1);
    assert_eq!(vault.debt_balance, u64::MAX);
    assert_eq!(vault.total_interest_accrued, 1);
    assert_eq!(vault.last_interest_accrued_at, MILLISECONDS_PER_HOUR);

    set_time(MILLISECONDS_PER_HOUR + 1);
    assert_eq!(vault_contract.accrue_interest_for_vault(&mut vault), Ok(()));
    assert_eq!(vault.borrowed_token_balance, u64::MAX - 1);
    assert_eq!(vault.debt_balance, u64::MAX);
    assert_eq!(vault.total_interest_accrued, 1);
    assert_eq!(vault.last_interest_accrued_at, 2 * MILLISECONDS_PER_HOUR);
}

#[ink::test]
fn accrue_interest_message_updates_stored_vault_balance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault_contract = TusdtVault::new_for_test(accounts.alice);

    set_time(0);
    let vault_id = create_vault_with_collateral(
        &mut vault_contract,
        accounts.alice,
        opening_collateral(1_000),
    );

    let mut stored_vault = vault_contract
        .get_vault(accounts.alice, vault_id)
        .expect("vault should exist");
    stored_vault.borrowed_token_balance = 100_000;
    stored_vault.debt_balance = 100_000;
    stored_vault.total_interest_accrued = 0;
    stored_vault.last_interest_accrued_at = 0;
    assert_eq!(
        vault_contract.save_vault(accounts.alice, vault_id, &stored_vault),
        Ok(())
    );

    set_time(30 * 24 * MILLISECONDS_PER_HOUR);
    assert_eq!(
        vault_contract.accrue_interest(accounts.alice, vault_id),
        Ok(100_826)
    );

    let updated_vault = vault_contract
        .get_vault(accounts.alice, vault_id)
        .expect("vault should still exist");
    assert_eq!(updated_vault.borrowed_token_balance, 100_000);
    assert_eq!(updated_vault.debt_balance, 100_826);
    assert_eq!(updated_vault.total_interest_accrued, 826);
    assert_eq!(vault_contract.get_total_debt(accounts.alice), 100_826);
    assert_eq!(
        updated_vault.last_interest_accrued_at,
        (30 * 24 + 1) * MILLISECONDS_PER_HOUR
    );
}

#[ink::test]
fn accrue_interest_message_rejects_missing_vault() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault_contract = TusdtVault::new_for_test(accounts.alice);

    assert_eq!(
        vault_contract.accrue_interest(accounts.alice, 0),
        Err(Error::VaultNotFound)
    );
}

#[ink::test]
fn total_debt_tracks_sum_of_owner_vault_debts() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault_contract = TusdtVault::new_for_test(accounts.alice);

    let alice_vault_0 =
        create_vault_with_collateral(&mut vault_contract, accounts.alice, opening_collateral(400));
    let alice_vault_1 =
        create_vault_with_collateral(&mut vault_contract, accounts.alice, opening_collateral(500));
    let bob_vault_0 =
        create_vault_with_collateral(&mut vault_contract, accounts.bob, opening_collateral(300));

    let mut alice_first = vault_contract
        .get_vault(accounts.alice, alice_vault_0)
        .expect("alice vault 0 should exist");
    alice_first.borrowed_token_balance = 125;
    alice_first.debt_balance = 135;
    alice_first.total_interest_accrued = 10;
    assert_eq!(
        vault_contract.save_vault(accounts.alice, alice_vault_0, &alice_first),
        Ok(())
    );

    let mut alice_second = vault_contract
        .get_vault(accounts.alice, alice_vault_1)
        .expect("alice vault 1 should exist");
    alice_second.borrowed_token_balance = 275;
    alice_second.debt_balance = 300;
    alice_second.total_interest_accrued = 25;
    assert_eq!(
        vault_contract.save_vault(accounts.alice, alice_vault_1, &alice_second),
        Ok(())
    );

    let mut bob_first = vault_contract
        .get_vault(accounts.bob, bob_vault_0)
        .expect("bob vault 0 should exist");
    bob_first.borrowed_token_balance = 80;
    bob_first.debt_balance = 83;
    bob_first.total_interest_accrued = 3;
    assert_eq!(
        vault_contract.save_vault(accounts.bob, bob_vault_0, &bob_first),
        Ok(())
    );

    assert_eq!(vault_contract.get_total_debt(accounts.alice), 435);
    assert_eq!(vault_contract.get_total_debt(accounts.bob), 83);

    alice_first.borrowed_token_balance = 160;
    alice_first.debt_balance = 172;
    alice_first.total_interest_accrued = 22;
    assert_eq!(
        vault_contract.save_vault(accounts.alice, alice_vault_0, &alice_first),
        Ok(())
    );

    assert_eq!(vault_contract.get_total_debt(accounts.alice), 472);
    assert_eq!(vault_contract.get_total_debt(accounts.bob), 83);
}

#[ink::test]
fn debt_payment_routes_interest_before_principal() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = Vault {
        id: 0,
        owner: accounts.alice,
        collateral_balance: 1_000,
        borrowed_token_balance: 110,
        debt_balance: 120,
        total_interest_accrued: 10,
        created_at: 0,
        last_interest_accrued_at: 0,
    };

    let payment = TusdtVault::apply_debt_payment(&mut vault, 6)
        .expect("partial repayment should be allocated successfully");
    assert_eq!(payment.principal_payment, 0);
    assert_eq!(payment.interest_payment, 6);
    assert_eq!(vault.borrowed_token_balance, 110);
    assert_eq!(vault.debt_balance, 114);
    assert_eq!(vault.total_interest_accrued, 10);

    let payment = TusdtVault::apply_debt_payment(&mut vault, 10)
        .expect("second repayment should cover remaining interest first");
    assert_eq!(payment.principal_payment, 6);
    assert_eq!(payment.interest_payment, 4);
    assert_eq!(vault.borrowed_token_balance, 104);
    assert_eq!(vault.debt_balance, 104);
    assert_eq!(vault.total_interest_accrued, 10);
}

#[ink::test]
fn debt_payment_rejects_amounts_above_outstanding_debt() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = Vault {
        id: 0,
        owner: accounts.alice,
        collateral_balance: 1_000,
        borrowed_token_balance: 50,
        debt_balance: 55,
        total_interest_accrued: 5,
        created_at: 0,
        last_interest_accrued_at: 0,
    };

    assert_eq!(
        TusdtVault::apply_debt_payment(&mut vault, 56),
        Err(Error::RepayAmountTooHigh)
    );
    assert_eq!(vault.borrowed_token_balance, 50);
    assert_eq!(vault.debt_balance, 55);
    assert_eq!(vault.total_interest_accrued, 5);
}

#[ink::test]
fn liquidatable_check_uses_liquidation_ratio_limit() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let vault_contract = TusdtVault::new_for_test(accounts.alice);
    let price = Ratio::from_integer(1);

    let safe_vault = Vault {
        id: 0,
        owner: accounts.alice,
        collateral_balance: 120,
        borrowed_token_balance: 100,
        debt_balance: 100,
        total_interest_accrued: 0,
        created_at: 0,
        last_interest_accrued_at: 0,
    };
    let unsafe_vault = Vault {
        debt_balance: 101,
        ..safe_vault.clone()
    };

    assert_eq!(
        vault_contract.is_liquidatable(price, &safe_vault),
        Ok(false)
    );
    assert_eq!(
        vault_contract.is_liquidatable(price, &unsafe_vault),
        Ok(true)
    );
}

fn install_tight_collateral_bounds(
    vault: &mut TusdtVault,
    governance: ink::primitives::AccountId,
    min_vault: u64,
    max_vault: u64,
    max_total: u64,
) {
    set_caller(governance);
    set_time(0);
    let params = VaultContractParamsConfig {
        min_vault_collateral: min_vault,
        max_vault_collateral: max_vault,
        max_total_collateral: max_total,
        ..test_contract_params()
    };
    assert_eq!(vault.set_contract_params(params), Ok(()));
    set_time(CONTRACT_PARAMS_TIMELOCK_MS);
    assert_eq!(vault.execute_contract_params_update(), Ok(()));
}

#[ink::test]
fn create_vault_rejects_when_exceeds_max_per_vault() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);
    install_tight_collateral_bounds(
        &mut vault,
        accounts.alice,
        TEST_MIN_VAULT_COLLATERAL,
        10_000_000,
        100_000_000,
    );

    set_caller(accounts.alice);
    transfer_in(10_000_001);
    assert_eq!(vault.create_vault(), Err(Error::CollateralCapExceeded));
    assert_eq!(vault.get_total_vaults_count(), 0);
    assert_eq!(vault.get_total_collateral_balance(), 0);
}

#[ink::test]
fn create_vault_rejects_when_exceeds_max_total() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);
    install_tight_collateral_bounds(
        &mut vault,
        accounts.alice,
        TEST_MIN_VAULT_COLLATERAL,
        10_000_000,
        15_000_000,
    );

    create_vault_with_collateral(&mut vault, accounts.alice, 10_000_000);

    set_caller(accounts.bob);
    transfer_in(6_000_000);
    assert_eq!(vault.create_vault(), Err(Error::CollateralCapExceeded));
    assert_eq!(vault.get_total_collateral_balance(), 10_000_000);
}

#[ink::test]
fn add_collateral_rejects_zero_amount() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    create_vault_with_collateral(&mut vault, accounts.alice, opening_collateral(100));

    set_caller(accounts.alice);
    set_transferred_value(0);
    assert_eq!(vault.add_collateral(0), Err(Error::InsufficientCollateral));
}

#[ink::test]
fn add_collateral_rejects_below_min_when_post_settlement_dust() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault_contract = TusdtVault::new_for_test(accounts.alice);

    let vault_id =
        create_vault_with_collateral(&mut vault_contract, accounts.alice, opening_collateral(100));

    let mut stored_vault = vault_contract
        .get_vault(accounts.alice, vault_id)
        .expect("vault should exist");
    stored_vault.collateral_balance = 100;
    assert_eq!(
        vault_contract.save_vault(accounts.alice, vault_id, &stored_vault),
        Ok(())
    );

    set_caller(accounts.alice);
    transfer_in(50);
    assert_eq!(
        vault_contract.add_collateral(vault_id),
        Err(Error::InsufficientCollateral)
    );
}

#[ink::test]
fn add_collateral_rejects_when_exceeds_max_per_vault() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);
    install_tight_collateral_bounds(
        &mut vault,
        accounts.alice,
        TEST_MIN_VAULT_COLLATERAL,
        10_000_000,
        100_000_000,
    );

    let vault_id = create_vault_with_collateral(&mut vault, accounts.alice, 9_000_000);

    set_caller(accounts.alice);
    transfer_in(2_000_000);
    assert_eq!(
        vault.add_collateral(vault_id),
        Err(Error::CollateralCapExceeded)
    );
}

#[ink::test]
fn add_collateral_rejects_when_exceeds_max_total() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);
    install_tight_collateral_bounds(
        &mut vault,
        accounts.alice,
        TEST_MIN_VAULT_COLLATERAL,
        10_000_000,
        15_000_000,
    );

    let alice_vault_id = create_vault_with_collateral(&mut vault, accounts.alice, 8_000_000);
    create_vault_with_collateral(&mut vault, accounts.bob, 6_000_000);

    set_caller(accounts.alice);
    transfer_in(2_000_000);
    assert_eq!(
        vault.add_collateral(alice_vault_id),
        Err(Error::CollateralCapExceeded)
    );
}

#[ink::test]
fn release_collateral_still_allowed_below_min_when_no_debt() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault_contract = TusdtVault::new_for_test(accounts.alice);

    let collateral = opening_collateral(0);
    let vault_id =
        create_vault_with_collateral(&mut vault_contract, accounts.alice, collateral);

    set_caller(accounts.alice);
    let result = vault_contract.release_collateral(vault_id, collateral - 1);
    assert_ne!(result, Err(Error::InsufficientCollateral));
}

#[ink::test]
fn governance_can_update_collateral_bounds() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);

    install_tight_collateral_bounds(
        &mut vault,
        accounts.alice,
        7_500_000,
        50_000_000,
        500_000_000,
    );

    let params = vault.get_contract_params();
    assert_eq!(params.min_vault_collateral, 7_500_000);
    assert_eq!(params.max_vault_collateral, 50_000_000);
    assert_eq!(params.max_total_collateral, 500_000_000);
}

#[ink::test]
fn validate_contract_params_rejects_inverted_bounds() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut vault = TusdtVault::new_for_test(accounts.alice);
    let valid = test_contract_params();

    set_caller(accounts.alice);

    assert_eq!(
        vault.set_contract_params(VaultContractParamsConfig {
            min_vault_collateral: 0,
            ..valid
        }),
        Err(Error::InvalidRatio)
    );

    assert_eq!(
        vault.set_contract_params(VaultContractParamsConfig {
            min_vault_collateral: 10_000_000,
            max_vault_collateral: 5_000_000,
            ..valid
        }),
        Err(Error::InvalidRatio)
    );

    assert_eq!(
        vault.set_contract_params(VaultContractParamsConfig {
            max_vault_collateral: 100_000_000,
            max_total_collateral: 50_000_000,
            ..valid
        }),
        Err(Error::InvalidRatio)
    );
}
