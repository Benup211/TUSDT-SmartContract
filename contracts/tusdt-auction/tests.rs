use super::auction::*;
use super::*;
use tusdt_primitives::Ratio;

fn set_caller(caller: ink::primitives::AccountId) {
    let callee = ink::env::account_id::<tusdt_env::CustomEnvironment>();
    ink::env::test::set_callee::<tusdt_env::CustomEnvironment>(callee);
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(caller);
}

fn set_time(timestamp: u64) {
    ink::env::test::set_block_timestamp::<tusdt_env::CustomEnvironment>(timestamp);
}

fn create_default_auction(
    contract: &mut TusdtAuction,
    vault_owner: ink::primitives::AccountId,
    vault_id: u32,
) -> u32 {
    contract
        .create_auction(
            vault_owner,
            vault_id,
            1_000,
            500,
            550,
            Ratio::from_integer(2),
            Some(1_000),
        )
        .expect("create_auction should succeed")
}

fn default_bid_metadata(hot_key: ink::primitives::AccountId) -> Option<BidMetadata> {
    Some(BidMetadata { hot_key })
}

#[ink::test]
fn new_works() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    assert_eq!(auction.controller(), accounts.alice);
    assert_eq!(auction.governance(), accounts.bob);
    assert_eq!(auction.admin(), None);
    assert_eq!(auction.get_total_auctions_count(), 0);
    assert_eq!(auction.get_active_auctions_count(), 0);
}

#[ink::test]
fn create_auction_works() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_time(10);
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    let auction_id = create_default_auction(&mut auction, accounts.bob, 7);
    assert_eq!(auction_id, 0);
    assert_eq!(auction.get_total_auctions_count(), 1);
    assert_eq!(auction.get_active_auctions_count(), 1);
    assert_eq!(
        auction.get_active_vault_auction(accounts.bob, 7),
        Some(auction_id)
    );

    let created = auction
        .get_auction(auction_id)
        .expect("auction should exist");
    assert_eq!(created.vault_owner, accounts.bob);
    assert_eq!(created.vault_id, 7);
    assert_eq!(created.collateral_balance, 1_000);
    assert_eq!(created.debt_balance, 500);
    assert_eq!(created.min_bid, 550);
    assert_eq!(created.liquidation_price, Ratio::from_integer(2));
    assert_eq!(created.starts_at, 10);
    assert_eq!(created.ends_at, 1_010);
    assert_eq!(created.highest_bid, 0);
    assert_eq!(created.highest_bidder, None);
    assert!(!created.is_finalized);
}

#[ink::test]
fn create_auction_fails_for_non_controller() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    set_caller(accounts.bob);
    assert_eq!(
        auction.create_auction(
            accounts.bob,
            1,
            1_000,
            400,
            440,
            Ratio::from_integer(2),
            Some(1_000)
        ),
        Err(Error::NotController)
    );
    assert_eq!(auction.get_total_auctions_count(), 0);
    assert_eq!(auction.get_active_auctions_count(), 0);
}

#[ink::test]
fn create_auction_fails_if_active_auction_exists_for_vault() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    create_default_auction(&mut auction, accounts.bob, 1);
    assert_eq!(
        auction.create_auction(
            accounts.bob,
            1,
            2_000,
            800,
            880,
            Ratio::from_integer(2),
            Some(1_000)
        ),
        Err(Error::AuctionAlreadyExistsForVault)
    );
}

#[ink::test]
fn create_auction_fails_on_invalid_duration() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    assert_eq!(
        auction.create_auction(
            accounts.bob,
            1,
            1_000,
            400,
            440,
            Ratio::from_integer(2),
            Some(0)
        ),
        Err(Error::InvalidDuration)
    );
    assert_eq!(
        auction.create_auction(
            accounts.bob,
            1,
            1_000,
            400,
            440,
            Ratio::from_integer(2),
            Some(604_800_001),
        ),
        Err(Error::InvalidDuration)
    );
}

#[ink::test]
fn place_bid_fails_when_auction_not_found() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    assert_eq!(
        auction.place_bid(42, 600, default_bid_metadata(accounts.django)),
        Err(Error::AuctionNotFound)
    );
}

#[ink::test]
fn place_bid_fails_when_bid_is_below_min_bid() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    let auction_id = create_default_auction(&mut auction, accounts.bob, 2);
    assert_eq!(
        auction.place_bid(auction_id, 549, default_bid_metadata(accounts.django)),
        Err(Error::BidBelowMinBid)
    );
}

#[ink::test]
fn place_bid_fails_for_non_admin_after_auction_end_without_bids() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_time(100);
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);
    set_caller(accounts.bob);
    assert_eq!(auction.set_admin(Some(accounts.bob)), Ok(()));
    set_caller(accounts.alice);

    let auction_id = create_default_auction(&mut auction, accounts.bob, 2);
    set_time(1_100);
    set_caller(accounts.django);
    assert_eq!(
        auction.place_bid(auction_id, 600, default_bid_metadata(accounts.django)),
        Err(Error::NotAdmin)
    );
}

#[ink::test]
fn finalize_auction_fails_without_bids_after_end() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_time(200);
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    let auction_id = create_default_auction(&mut auction, accounts.bob, 3);
    set_time(1_200);
    assert_eq!(
        auction.finalize_auction(auction_id),
        Err(Error::AuctionHasNoBids)
    );

    let pending = auction
        .get_auction(auction_id)
        .expect("auction should exist");
    assert!(!pending.is_finalized);
    assert_eq!(auction.get_active_auctions_count(), 1);
    assert_eq!(
        auction.get_active_vault_auction(accounts.bob, 3),
        Some(auction_id)
    );
}

#[ink::test]
fn late_bid_is_allowed_only_for_admin_when_no_bids_exist() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction_contract = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);
    set_caller(accounts.bob);
    assert_eq!(auction_contract.set_admin(Some(accounts.bob)), Ok(()));
    let auction = Auction {
        id: 0,
        vault_owner: accounts.bob,
        vault_id: 9,
        collateral_balance: 1_000,
        debt_balance: 500,
        min_bid: 550,
        liquidation_price: Ratio::from_integer(2),
        starts_at: 200,
        ends_at: 1_200,
        highest_bidder: None,
        highest_bid: 0,
        highest_bid_id: None,
        bid_count: 0,
        is_finalized: false,
    };

    set_time(1_200);
    assert_eq!(
        auction_contract.ensure_bid_allowed(&auction, accounts.bob),
        Ok(())
    );
    assert_eq!(
        auction_contract.ensure_bid_allowed(&auction, accounts.django),
        Err(Error::NotAdmin)
    );
}

#[ink::test]
fn late_bid_fails_after_end_once_bids_exist() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let auction_contract = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);
    let auction = Auction {
        id: 0,
        vault_owner: accounts.bob,
        vault_id: 9,
        collateral_balance: 1_000,
        debt_balance: 500,
        min_bid: 550,
        liquidation_price: Ratio::from_integer(2),
        starts_at: 200,
        ends_at: 1_200,
        highest_bidder: Some(accounts.eve),
        highest_bid: 550,
        highest_bid_id: Some(0),
        bid_count: 1,
        is_finalized: false,
    };

    set_time(1_200);
    assert_eq!(
        auction_contract.ensure_bid_allowed(&auction, accounts.bob),
        Err(Error::AuctionEnded)
    );
}

#[ink::test]
fn finalize_auction_fails_before_end() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_time(50);
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    let auction_id = create_default_auction(&mut auction, accounts.bob, 4);
    set_time(999);
    assert_eq!(
        auction.finalize_auction(auction_id),
        Err(Error::AuctionNotEnded)
    );
}

#[ink::test]
fn governance_sets_admin() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    set_caller(accounts.django);
    assert_eq!(
        auction.set_admin(Some(accounts.eve)),
        Err(Error::NotGovernance)
    );

    set_caller(accounts.bob);
    assert_eq!(auction.set_admin(Some(accounts.eve)), Ok(()));
    assert_eq!(auction.admin(), Some(accounts.eve));
}

#[ink::test]
fn controller_updates_auction_governance() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    set_caller(accounts.bob);
    assert_eq!(
        auction.update_governance(accounts.django),
        Err(Error::NotController)
    );

    set_caller(accounts.alice);
    assert_eq!(auction.update_governance(accounts.django), Ok(()));
    assert_eq!(auction.governance(), accounts.django);

    set_caller(accounts.bob);
    assert_eq!(
        auction.set_admin(Some(accounts.eve)),
        Err(Error::NotGovernance)
    );

    set_caller(accounts.django);
    assert_eq!(auction.set_admin(Some(accounts.eve)), Ok(()));
    assert_eq!(auction.admin(), Some(accounts.eve));
}

#[ink::test]
fn finalize_auction_fails_if_already_finalized() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_time(100);
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    let auction_id = create_default_auction(&mut auction, accounts.bob, 5);
    auction
        .seed_bid_for_test(auction_id, accounts.eve, 600)
        .expect("test setup should seed a bid");
    set_time(1_100);
    assert_eq!(auction.finalize_auction(auction_id), Ok(()));
    assert_eq!(
        auction.finalize_auction(auction_id),
        Err(Error::AuctionFinalized)
    );
}

#[ink::test]
fn transfer_winning_bid_fails_for_non_controller() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_time(100);
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    let auction_id = create_default_auction(&mut auction, accounts.bob, 6);
    auction
        .seed_bid_for_test(auction_id, accounts.eve, 600)
        .expect("test setup should seed a bid");
    set_time(1_100);
    assert_eq!(auction.finalize_auction(auction_id), Ok(()));

    set_caller(accounts.bob);
    assert_eq!(
        auction.transfer_winning_bid(auction_id, accounts.django),
        Err(Error::NotController)
    );
}

#[ink::test]
fn transfer_winning_bid_fails_before_finalize() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    let auction_id = create_default_auction(&mut auction, accounts.bob, 7);
    auction
        .seed_bid_for_test(auction_id, accounts.eve, 600)
        .expect("test setup should seed a bid");

    assert_eq!(
        auction.transfer_winning_bid(auction_id, accounts.django),
        Err(Error::AuctionNotEnded)
    );
}

#[ink::test]
fn get_all_auctions_supports_pagination() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    for vault_id in 0..12 {
        assert_eq!(
            auction.create_auction(
                accounts.bob,
                vault_id,
                1_000,
                500,
                550,
                Ratio::from_integer(2),
                Some(2_000),
            ),
            Ok(vault_id)
        );
    }

    let page0 = auction.get_all_auctions(0).expect("page 0 should exist");
    assert_eq!(page0.len(), 10);
    assert_eq!(page0[0].id, 0);
    assert_eq!(page0[9].id, 9);

    let page1 = auction.get_all_auctions(1).expect("page 1 should exist");
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].id, 10);
    assert_eq!(page1[1].id, 11);

    assert!(matches!(
        auction.get_all_auctions(2),
        Ok(auctions) if auctions.is_empty()
    ));
}

#[ink::test]
fn get_active_auctions_updates_after_finalize() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    set_time(1);
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    let first = create_default_auction(&mut auction, accounts.bob, 11);
    let second = create_default_auction(&mut auction, accounts.bob, 12);
    auction
        .seed_bid_for_test(first, accounts.eve, 600)
        .expect("test setup should seed a bid");
    assert_eq!(first, 0);
    assert_eq!(second, 1);

    set_time(2_000);
    assert_eq!(auction.finalize_auction(first), Ok(()));

    let active = auction
        .get_active_auctions(0)
        .expect("active auctions should have one entry");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, second);
    assert_eq!(auction.get_active_auctions_count(), 1);
}

#[ink::test]
fn get_bids_returns_empty_for_out_of_bounds_page_when_no_bids() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    let auction_id = create_default_auction(&mut auction, accounts.bob, 13);
    assert!(matches!(
        auction.get_bids(auction_id, 0),
        Ok(bids) if bids.is_empty()
    ));
}

#[ink::test]
fn get_auction_bid_returns_bid_for_bidder() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    let auction_id = create_default_auction(&mut auction, accounts.bob, 14);
    auction
        .seed_bid_for_test(auction_id, accounts.django, 600)
        .expect("test setup should seed a bid");

    let bid = auction
        .get_auction_bid(auction_id, accounts.django)
        .expect("bid should exist");
    assert_eq!(bid.id, 0);
    assert_eq!(bid.auction_id, auction_id);
    assert_eq!(bid.bidder, accounts.django);
    assert_eq!(bid.amount, 600);

    assert!(auction.get_auction_bid(auction_id, accounts.eve).is_none());
}

#[ink::test]
fn withdraw_refund_fails_when_bid_not_found() {
    let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
    let mut auction = TusdtAuction::new(accounts.alice, accounts.bob, accounts.charlie);

    let auction_id = create_default_auction(&mut auction, accounts.bob, 15);
    assert_eq!(
        auction.withdraw_refund(auction_id, 0),
        Err(Error::BidNotFound)
    );
}
