# tusdt-auction — Liquidation Auction

The Auction contract runs the TUSDT protocol's liquidation auctions: when a vault goes underwater, its collateral is sold here to the highest bidder to repay the vault's debt. Think of it as MakerDAO's collateral auction (the *Flipper*): open, increasing TUSDT bids, the highest bid wins when time runs out, losers get their money back.

The contract is **controller-gated**: only the vault contract can create auctions and claim winning bids; everyone else bids, finalizes, and withdraws refunds permissionlessly. Auctions sell **native TAO** (the vault's unstaked alpha) for **TUSDT**.

## How it works

**One active auction per vault.** `create_auction` is vault-only and reverts with `AuctionAlreadyExistsForVault` if the vault already has an open auction. The vault supplies the auction's `collateral_balance` (TAO recovered), `debt_balance` (TUSDT to repay), `min_bid`, the liquidation-time price snapshot, and an optional duration.

**Clock.** `ends_at = block_timestamp + duration_ms`. The vault passes its global `auction_duration_ms` (default 1 hour); the auction's own fallback default is also 3,600,000 ms, capped at 7 days (`InvalidDuration` if zero or above max).

**Ascending bids with pre-approval.** Bidding is permissionless while `now < ends_at`. Bids are TUSDT and pulled with the PSP22 `transfer_from` pattern — you must `approve` the auction contract first:

- New bid must be ≥ `min_bid` (`BidBelowMinBid`), where the vault sets `min_bid = debt × (1 + liquidation_fee)`.
- Each bidder holds **one bid record per auction**; re-bidding must strictly increase your amount (`BidAmountNotIncreased`) and pulls only the *delta* (`bid_amount − previous_amount`).
- The highest bid is tracked in `highest_bidder` / `highest_bid` on the auction record.

**Admin backstop.** If an auction expires with **zero bids**, normal bidding is closed (`AuctionEnded`) but the configured `admin` account may still place a bid (`NotAdmin` for anyone else) so collateral never gets stranded.

**Finalize → settle → refund.**

1. After `ends_at`, **anyone** calls `finalize_auction(auction_id)`: requires ≥ 1 bid (`AuctionHasNoBids`), marks the auction finalized, and removes it from the active index. Re-calling returns `AuctionFinalized`.
2. The **vault** (controller) calls `settle_liquidation_auction(owner, vault_id)`, which pulls the winning bid via `transfer_winning_bid(auction_id, vault)` and distributes the collateral.
3. **Losing bidders** withdraw their full bid with `withdraw_refund(auction_id, bid_id)` — bidder-only (`NotBidder`), post-finalization only (`AuctionNotEnded`), and never for the winning bid (`WinningBidLocked`).

**Where the money goes** (vault-side settlement):

```text
winner receives      = collateral_sold − transaction_fee        # native TAO
treasury receives    = transaction_fee × collateral_sold        # native TAO (default 0.3%)
debt burned          = debt_balance at trigger                  # TUSDT
surplus TUSDT        = winning_bid − debt_balance               # stays in vault → treasury
```

The full winning bid is transferred to the vault, which burns exactly the auction's `debt_balance`; anything above that (at minimum the liquidation fee, since bids start at `debt × 1.11`) remains as TUSDT surplus in the vault, claimable by governance via `claim_surplus_tusdt`.

## User flow

1. **Approve TUSDT** — `token.approve(auction_address, bid_amount)` so the auction can pull your bid.
2. **Bid** — `place_bid(auction_id, bid_amount, metadata?)` while the auction is live. Optional metadata can carry your originating hotkey.
3. **Raise** — call `place_bid` again with a strictly higher amount; only the difference is pulled.
4. **Wait for `ends_at`**, then anyone calls `finalize_auction(auction_id)`.
5. **The vault settles** — the winner receives the TAO collateral (minus the 0.3% transaction fee) in the vault's `settle_liquidation_auction`.
6. **Losing bidders** call `withdraw_refund(auction_id, bid_id)` to get their TUSDT back.

## Key parameters

| Parameter | Meaning | Default | Scale |
|---|---|---|---|
| `DEFAULT_AUCTION_DURATION_MS` | Fallback duration if the vault passes `None` | 3,600,000 (1 h) | ms |
| `MAX_AUCTION_DURATION_MS` | Maximum allowed duration | 604,800,000 (7 d) | ms |
| `min_bid` | Minimum first bid, set by the vault | `debt × (1 + liquidation_fee)` (e.g. debt × 1.11) | TUSDT units (u64, 9 decimals) |
| `PAGE_SIZE` | Page size for `get_bids` / `get_all_auctions` / `get_active_auctions` | 10 | entries |

**Scale conventions.** All amounts are `u64` balances with 9 decimals (1 TUSDT = 1e9 units; TAO collateral in rao, 1 TAO = 1e9 rao). Timestamps are Unix epoch **milliseconds**. `liquidation_price` is the vault's 1e18 fixed-point price snapshot at trigger time.

## Talks to

- **tusdt-erc20**: `transfer_from` (bidder → auction) on every bid; `transfer` for winning-bid payout and refunds.
- **tusdt-vault-alpha (controller)**: creates auctions (`create_auction`), reads them (`get_auction`), and pulls the winning bid (`transfer_winning_bid`). Governance hand-off via `update_governance` / `set_controller`.
- **tusdt-governance**: updates `admin` (`set_admin`) and the controller during upgrades (`set_controller`).

## Errors

All 19 variants are catalogued in the [error reference](../errors/auction.md). Notable ones:

- `NotController` — only the vault may create auctions / claim winning bids.
- `AuctionAlreadyExistsForVault` — that vault already has an open auction.
- `BidBelowMinBid` — first bid must clear the minimum.
- `BidAmountNotIncreased` — re-bids must strictly exceed your previous amount.
- `AuctionEnded` — the auction is closed (or open only to admin when it has no bids).
- `AuctionNotEnded` — finalize/refund/transfer before `ends_at`.
- `WinningBidLocked` — the winning bid isn't refundable; losers only.
- `AuctionHasNoBids` — ended with zero bids; only the admin backstop can still bid.
