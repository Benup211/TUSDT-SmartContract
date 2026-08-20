#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::tusdt::*;
use super::*;
use tusdt_test_support::set_caller;

#[ink::test]
fn new_works() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let erc20 = TusdtErc20::new(accounts.alice);

    assert_eq!(erc20.controller(), accounts.alice);
    assert_eq!(erc20.total_supply(), 0);
    assert_eq!(erc20.balance_of(accounts.alice), 0);
}

#[ink::test]
fn total_supply_works() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);

    assert_eq!(erc20.mint(accounts.alice, 100), Ok(()));
    assert_eq!(erc20.mint(accounts.bob, 100), Ok(()));
    assert_eq!(erc20.total_supply(), 200);
}

#[ink::test]
fn balance_of_works() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);
    assert_eq!(erc20.mint(accounts.alice, 100), Ok(()));

    assert_eq!(erc20.balance_of(accounts.alice), 100);
    assert_eq!(erc20.balance_of(accounts.bob), 0);
}

#[ink::test]
fn transfer_works() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);
    assert_eq!(erc20.mint(accounts.alice, 100), Ok(()));

    assert_eq!(erc20.balance_of(accounts.bob), 0);
    assert_eq!(erc20.transfer(accounts.bob, 10), Ok(()));
    assert_eq!(erc20.balance_of(accounts.bob), 10);
    assert_eq!(erc20.balance_of(accounts.alice), 90);
}

#[ink::test]
fn invalid_transfer_should_fail() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);
    assert_eq!(erc20.mint(accounts.alice, 100), Ok(()));

    assert_eq!(erc20.balance_of(accounts.bob), 0);

    set_caller(accounts.bob);

    assert_eq!(erc20.transfer(accounts.eve, 10), Err(Error::InsufficientBalance));
    assert_eq!(erc20.balance_of(accounts.alice), 100);
    assert_eq!(erc20.balance_of(accounts.bob), 0);
    assert_eq!(erc20.balance_of(accounts.eve), 0);
}

#[ink::test]
fn transfer_from_works() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);
    assert_eq!(erc20.mint(accounts.alice, 100), Ok(()));

    assert_eq!(
        erc20.transfer_from(accounts.alice, accounts.eve, 10),
        Err(Error::InsufficientAllowance)
    );
    assert_eq!(erc20.approve(accounts.bob, 10), Ok(()));

    set_caller(accounts.bob);

    assert_eq!(erc20.transfer_from(accounts.alice, accounts.eve, 10), Ok(()));
    assert_eq!(erc20.balance_of(accounts.eve), 10);
    assert_eq!(erc20.allowance(accounts.alice, accounts.bob), 0);
}

#[ink::test]
fn allowance_must_not_change_on_failed_transfer() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);
    assert_eq!(erc20.mint(accounts.alice, 100), Ok(()));

    let alice_balance = erc20.balance_of(accounts.alice);
    let initial_allowance = alice_balance + 2;
    assert_eq!(erc20.approve(accounts.bob, initial_allowance), Ok(()));

    set_caller(accounts.bob);

    assert_eq!(
        erc20.transfer_from(accounts.alice, accounts.eve, alice_balance + 1),
        Err(Error::InsufficientBalance)
    );
    assert_eq!(erc20.allowance(accounts.alice, accounts.bob), initial_allowance);
}

#[ink::test]
fn mint_fails_for_non_minter() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);

    set_caller(accounts.bob);

    assert_eq!(erc20.mint(accounts.bob, 100), Err(Error::NotMinter));
    assert_eq!(erc20.total_supply(), 0);
    assert_eq!(erc20.balance_of(accounts.bob), 0);
}

#[ink::test]
fn burn_works_for_controller() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);
    assert_eq!(erc20.mint(accounts.bob, 100), Ok(()));

    assert_eq!(erc20.burn(accounts.bob, 40), Ok(()));
    assert_eq!(erc20.total_supply(), 60);
    assert_eq!(erc20.balance_of(accounts.bob), 60);
}

#[ink::test]
fn burn_fails_for_non_minter() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);
    assert_eq!(erc20.mint(accounts.bob, 100), Ok(()));

    set_caller(accounts.bob);

    assert_eq!(erc20.burn(accounts.bob, 10), Err(Error::NotMinter));
    assert_eq!(erc20.total_supply(), 100);
    assert_eq!(erc20.balance_of(accounts.bob), 100);
}

#[ink::test]
fn burn_fails_on_insufficient_balance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);
    assert_eq!(erc20.mint(accounts.bob, 50), Ok(()));

    assert_eq!(erc20.burn(accounts.bob, 60), Err(Error::InsufficientBalance));
    assert_eq!(erc20.total_supply(), 50);
    assert_eq!(erc20.balance_of(accounts.bob), 50);
}

#[ink::test]
fn approve_overwrites_allowance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);

    assert_eq!(erc20.approve(accounts.bob, 10), Ok(()));
    assert_eq!(erc20.allowance(accounts.alice, accounts.bob), 10);

    assert_eq!(erc20.approve(accounts.bob, 3), Ok(()));
    assert_eq!(erc20.allowance(accounts.alice, accounts.bob), 3);
}

#[ink::test]
fn increase_allowance_adds_to_existing_allowance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);

    assert_eq!(erc20.approve(accounts.bob, 10), Ok(()));
    assert_eq!(erc20.increase_allowance(accounts.bob, 7), Ok(()));
    assert_eq!(erc20.allowance(accounts.alice, accounts.bob), 17);
}

#[ink::test]
fn increase_allowance_saturates_at_balance_max() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);
    let max_balance = <tusdt_env::CustomEnvironment as ink::env::Environment>::Balance::MAX;

    assert_eq!(erc20.approve(accounts.bob, max_balance - 2), Ok(()));
    assert_eq!(erc20.increase_allowance(accounts.bob, 10), Ok(()));
    assert_eq!(erc20.allowance(accounts.alice, accounts.bob), max_balance);
}

#[ink::test]
fn decrease_allowance_subtracts_from_existing_allowance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);

    assert_eq!(erc20.approve(accounts.bob, 10), Ok(()));
    assert_eq!(erc20.decrease_allowance(accounts.bob, 4), Ok(()));
    assert_eq!(erc20.allowance(accounts.alice, accounts.bob), 6);
}

#[ink::test]
fn decrease_allowance_fails_when_delta_exceeds_allowance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);

    assert_eq!(erc20.approve(accounts.bob, 3), Ok(()));
    assert_eq!(erc20.decrease_allowance(accounts.bob, 4), Err(Error::InsufficientAllowance));
    assert_eq!(erc20.allowance(accounts.alice, accounts.bob), 3);
}

#[ink::test]
fn transfer_from_partially_consumes_allowance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut erc20 = TusdtErc20::new(accounts.alice);
    assert_eq!(erc20.mint(accounts.alice, 100), Ok(()));
    assert_eq!(erc20.approve(accounts.bob, 30), Ok(()));

    set_caller(accounts.bob);

    assert_eq!(erc20.transfer_from(accounts.alice, accounts.eve, 10), Ok(()));
    assert_eq!(erc20.allowance(accounts.alice, accounts.bob), 20);
    assert_eq!(erc20.balance_of(accounts.eve), 10);
}
