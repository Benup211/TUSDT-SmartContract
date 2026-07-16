use super::otc::*;
use ink::env::test;
use tusdt_env::StakeInfo;

fn default_accounts() -> test::DefaultAccounts<tusdt_env::CustomEnvironment> {
    test::default_accounts::<tusdt_env::CustomEnvironment>()
}

fn setup() -> (TusdtOtc, test::DefaultAccounts<tusdt_env::CustomEnvironment>) {
    let accounts = default_accounts();
    let otc = TusdtOtc::new_for_test(accounts.alice);
    (otc, accounts)
}

// ── Chain extension mock ───────────────────────────────────────────

struct MockExtension {
    stake: Option<u64>,
    should_fail: bool,
    transfer_fails: bool,
}

impl test::ChainExtension for MockExtension {
    fn ext_id(&self) -> u16 { 0x1000 }

    fn call(&mut self, func_id: u16, _input: &[u8], output: &mut Vec<u8>) -> u32 {
        if self.should_fail { return 1; }
        match func_id {
            0 => {
                let info = self.stake.map(|s| StakeInfo {
                    hotkey: default_accounts().bob,
                    coldkey: default_accounts().alice,
                    netuid: ink::scale::Compact(1),
                    stake: ink::scale::Compact(s),
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
                let price: u64 = 1_000_000_000;
                ink::scale::Encode::encode_to(&price, output);
                0
            }
            2 => 0,   // remove_stake success
            6 => {
                if self.transfer_fails { 2 } else { 0 }
            }
            _ => 1,
        }
    }
}

fn register_mock(stake: u64) {
    test::register_chain_extension(MockExtension { stake: Some(stake), should_fail: false, transfer_fails: false });
}

fn register_mock_failing_transfer(stake: u64) {
    test::register_chain_extension(MockExtension { stake: Some(stake), should_fail: false, transfer_fails: true });
}

// ── Basic tests ────────────────────────────────────────────────────

#[ink::test]
fn constructor_sets_owner() {
    let (otc, accounts) = setup();
    assert_eq!(otc.owner(), accounts.alice);
    assert!(!otc.is_paused());
}

#[ink::test]
fn owner_can_pause_unpause() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.pause().unwrap();
    assert!(otc.is_paused());
    otc.unpause().unwrap();
    assert!(!otc.is_paused());
}

#[ink::test]
fn non_owner_cannot_pause() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.bob);
    assert!(otc.pause().is_err());
}

#[ink::test]
fn deposit_alpha_registers_intent() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(1_000_000, 1).unwrap();
}

#[ink::test]
fn deposit_alpha_rejects_zero() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    assert!(otc.deposit_alpha(0, 1).is_err());
}

#[ink::test]
fn deposit_alpha_rejects_duplicate() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(1_000_000, 1).unwrap();
    assert!(otc.deposit_alpha(500_000, 1).is_err());
}

#[ink::test]
fn create_order_rejects_zero_price() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    assert!(otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 0, 1_000_000).is_err());
}

#[ink::test]
fn paused_blocks_trading() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.pause().unwrap();
    assert!(otc.deposit_alpha(1_000_000, 1).is_err());
}

#[ink::test]
fn cancel_order_fails_for_non_maker() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(1_000_000, 1).unwrap();
    register_mock(1_000_000);
    let id = otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 1_000_000).unwrap();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.bob);
    assert_eq!(otc.cancel_order(id), Err(Error::NotMaker));
}

#[ink::test]
fn fulfill_order_rejects_maker_as_taker() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(1_000_000, 1).unwrap();
    register_mock(1_000_000);
    let id = otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 1_000_000).unwrap();
    assert_eq!(otc.fulfill_order(id), Err(Error::CannotFulfillOwnOrder));
}

#[ink::test]
fn get_order_returns_none_for_unknown_id() {
    let (otc, _accounts) = setup();
    assert!(otc.get_order(999).is_none());
}

// ═══════════════════════════════════════════════════════════════════
// SCENARIO 1: Deposit without order — only depositor key can use it
// ═══════════════════════════════════════════════════════════════════

#[ink::test]
fn scenario1_deposit_only_depositor_can_use() {
    let (mut otc, accounts) = setup();

    // Alice deposits alpha intent on subnet 1
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(5_000_000, 1).unwrap();

    // Alice's pending deposit is keyed to (Alice,1). Duplicate deposit
    // proves the original entry is still tracked and blocks new entries.
    assert!(otc.deposit_alpha(1_000, 1).is_err(),
        "duplicate deposit blocked: (Alice,1) still active");

    // Bob cannot consume Alice's deposit — his key won't match the entry
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.bob);
    register_mock(5_000_000);
    let result = otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 1_000);
    assert_eq!(result, Err(Error::InsufficientCollateral),
        "Bob's create_order finds no (Bob,1) pending_deposit — Alice's deposit is safe");
}

// ═══════════════════════════════════════════════════════════════════
// SCENARIO 2: Cancel order — same key required, order preserved
// ═══════════════════════════════════════════════════════════════════

#[ink::test]
fn scenario2_cancel_order_same_key_only() {
    let (mut otc, accounts) = setup();

    // Alice creates a Sell order (Native collateral, wants TUSDT)
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(5_000_000, 1).unwrap();
    register_mock(5_000_000);
    let order_id = otc.create_order(
        1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 5_000_000
    ).unwrap();
    assert_eq!(otc.get_order(order_id).unwrap().status, OrderStatus::Active);

    // Bob cannot cancel Alice's order
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.bob);
    assert_eq!(otc.cancel_order(order_id), Err(Error::NotMaker),
        "wrong key rejected for cancellation");

    // Alice cancels — alpha returned via transfer_stake (needs mock)
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    register_mock(5_000_000);
    otc.cancel_order(order_id).unwrap();
    assert_eq!(otc.get_order(order_id).unwrap().status, OrderStatus::Cancelled);

    // Verify cancelled order cannot be fulfilled
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.bob);
    assert_eq!(otc.fulfill_order(order_id), Err(Error::OrderNotActive),
        "cancelled order rejects fulfillment");
}

// ═══════════════════════════════════════════════════════════════════
// SCENARIO 3: Fulfillment with transfer failure — order preserved
// ═══════════════════════════════════════════════════════════════════

#[ink::test]
fn scenario3_fulfill_transfer_fails_order_stays_active() {
    let (mut otc, accounts) = setup();

    // Alice: Sell order (alpha → Native TAO).  Use Native for counter_collateral
    // to avoid cross-contract ERC20 calls (not supported in off-chain engine).
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(10_000_000, 1).unwrap();
    register_mock(10_000_000);
    let order_id = otc.create_order(
        1, OrderSide::Sell, Collateral::Native, Collateral::Native, 10_000, 10_000_000
    ).unwrap();

    // Bob tries to fulfill — transfer_stake fails.  Bob "sends" TAO.
    register_mock_failing_transfer(10_000_000);
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.bob);
    ink::env::test::set_value_transferred::<ink::env::DefaultEnvironment>(10_000_000_000);
    let result = otc.fulfill_order(order_id);
    assert!(result.is_err(), "fulfill fails when transfer_stake fails");

    // CRITICAL: Order stays Active (not Fulfilled, not stuck in limbo)
    let order = otc.get_order(order_id).unwrap();
    assert_eq!(order.status, OrderStatus::Active,
        "Order remains Active after failed fulfillment — no partial state");

    // Order can still be cancelled (needs mock for alpha return)
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    register_mock(10_000_000);
    otc.cancel_order(order_id).unwrap();
    assert_eq!(otc.get_order(order_id).unwrap().status, OrderStatus::Cancelled);
}

// ═══════════════════════════════════════════════════════════════════
// PROBES
// ═══════════════════════════════════════════════════════════════════

#[ink::test]
fn probe_cannot_cancel_twice() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(5_000_000, 1).unwrap();
    register_mock(5_000_000);
    let id = otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 5_000_000).unwrap();
    otc.cancel_order(id).unwrap();
    assert_eq!(otc.cancel_order(id), Err(Error::OrderNotActive), "double cancel rejected");
}

#[ink::test]
fn probe_cancel_then_fulfill_rejected() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(5_000_000, 1).unwrap();
    register_mock(5_000_000);
    let id = otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 5_000_000).unwrap();
    otc.cancel_order(id).unwrap();

    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.bob);
    assert_eq!(otc.fulfill_order(id), Err(Error::OrderNotActive), "cannot fulfill cancelled");
}

#[ink::test]
fn probe_order_requires_collateral_amount_not_zero() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    assert!(otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 0).is_err());
}

// ── Reserved alpha tracking ──────────────────────────────────────────

#[ink::test]
fn sell_order_increments_reserved_alpha() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(1_000_000, 1).unwrap();
    register_mock(1_000_000);
    assert_eq!(otc.get_reserved_alpha(1), 0);
    otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 1_000_000).unwrap();
    assert_eq!(otc.get_reserved_alpha(1), 1_000_000);
}

#[ink::test]
fn cancel_sell_order_decrements_reserved_alpha() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(1_000_000, 1).unwrap();
    register_mock(1_000_000);
    let id = otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 1_000_000).unwrap();
    assert_eq!(otc.get_reserved_alpha(1), 1_000_000);
    otc.cancel_order(id).unwrap();
    assert_eq!(otc.get_reserved_alpha(1), 0);
}

#[ink::test]
fn get_reserved_alpha_returns_zero_for_unknown_netuid() {
    let (otc, _accounts) = setup();
    assert_eq!(otc.get_reserved_alpha(99), 0);
}

// ── claim_excess_alpha ───────────────────────────────────────────────

#[ink::test]
fn owner_can_claim_excess_alpha() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(1_000_000, 1).unwrap();
    register_mock(2_000_000);  // stake > reserved → 1_000_000 excess
    otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 1_000_000).unwrap();
    assert_eq!(otc.get_reserved_alpha(1), 1_000_000);

    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.claim_excess_alpha(1, accounts.bob).unwrap();
}

#[ink::test]
fn non_owner_cannot_claim_excess_alpha() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(1_000_000, 1).unwrap();
    register_mock(2_000_000);
    otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 1_000_000).unwrap();

    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.bob);
    assert_eq!(otc.claim_excess_alpha(1, accounts.bob), Err(Error::NotOwner));
}

#[ink::test]
fn claim_excess_alpha_noop_when_no_excess() {
    let (mut otc, accounts) = setup();
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    otc.deposit_alpha(1_000_000, 1).unwrap();
    register_mock(1_000_000);  // stake == reserved → no excess
    otc.create_order(1, OrderSide::Sell, Collateral::Native, Collateral::Tusdt, 10_000, 1_000_000).unwrap();

    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(accounts.alice);
    // Should succeed without error (no-op).
    otc.claim_excess_alpha(1, accounts.bob).unwrap();
}
