# Auction — Error catalog

**Contract:** `contracts/tusdt-auction/lib.rs` (package `tusdt-auction`, artifact `tusdt_auction`)

**Enum:** `pub enum Error` — all variants fieldless, `#[derive(Debug, PartialEq, Eq)]` + `#[ink::scale_derive(Encode, Decode, TypeInfo)]`. 19 variants.

**Enum doc comment (verbatim):**

> Errors returned by the auction contract.

Fieldless `Error` enum: each variant encodes as a single `u8` SCALE index. See [index.md](index.md) for how errors surface in dedot/dApp/CLI clients.

## Variants

### `NotController`

> Caller is not the controller (vault) contract.

**Returned by:** `ensure_controller` (helper)

**Client guidance:** Authorization: only the vault (controller) contract may call; report if a user call hits it.

### `NotGovernance`

> Caller is not the governance account.

**Returned by:** `ensure_governance` (helper)

**Client guidance:** Authorization: only governance may call; do not retry.

### `NotAdmin`

> Caller is not the configured admin.

**Returned by:** `ensure_bid_allowed` (helper)

**Client guidance:** Authorization: only the configured admin may call (the admin may bid after end when no bids exist).

### `AuctionNotFound`

> No auction found for the given ID.

**Returned by:** `finalize_auction` (message), `get_active_auctions` (message), `get_all_auctions` (message), `get_bids` (message), `place_bid` (message), `transfer_winning_bid` (message), `withdraw_refund` (message), `remove_active_auction` (helper), `seed_bid_for_test` (helper)

**Client guidance:** Check the auction id against `get_all_auctions` / `get_active_auctions` before acting; retry with a valid id.

### `BidNotFound`

> No bid found for the given (auction_id, bid_id) pair.

**Returned by:** `finalize_auction` (message), `get_bids` (message), `transfer_winning_bid` (message), `withdraw_refund` (message), `prepare_bid` (helper)

**Client guidance:** Check the (auction_id, bid_id) pair against `get_bids` before acting; retry with a valid pair.

### `NotBidder`

> Caller is not the original bidder of the referenced bid.

**Returned by:** `withdraw_refund` (message)

**Client guidance:** Only the original bidder may withdraw that bid's refund; check the connected account.

### `AuctionAlreadyExistsForVault`

> An active auction already exists for the given vault.

**Returned by:** `create_auction` (message)

**Client guidance:** Wait for the vault's active auction to end and be finalized, then retry.

### `BidBelowMinBid`

> Bid amount is below the auction's minimum.

**Returned by:** `prepare_bid` (helper)

**Client guidance:** Input error: raise the bid above the auction minimum and retry.

### `AuctionEnded`

> Auction has already ended and accepts no further bids.

**Returned by:** `ensure_bid_allowed` (helper)

**Client guidance:** State error: the auction is closed; do not retry the bid - wait for finalization / withdrawals.

### `AuctionNotEnded`

> Operation requires the auction to have ended.

**Returned by:** `finalize_auction` (message), `transfer_winning_bid` (message), `withdraw_refund` (message)

**Client guidance:** State error: wait until the auction end time before finalizing or claiming; retry later.

### `AuctionFinalized`

> Auction has already been finalized.

**Returned by:** `finalize_auction` (message), `ensure_bid_allowed` (helper)

**Client guidance:** Idempotency: already finalized - treat as success for finalize; expect the refund path for withdrawals.

### `AuctionHasNoBids`

> Auction ended with no valid bids.

**Returned by:** `finalize_auction` (message), `transfer_winning_bid` (message)

**Client guidance:** State error: nothing to settle - treat as a settled empty auction; do not retry.

### `WinningBidLocked`

> Refund is not available because this bid is the winning bid.

**Returned by:** `withdraw_refund` (message)

**Client guidance:** The winning bid is locked for the controller and is not refundable; no client action.

### `WinningBidAlreadyTransferred`

> The winning bid was already transferred out.

**Returned by:** `transfer_winning_bid` (message)

**Client guidance:** Idempotency: the payout already happened; treat as success.

### `InvalidDuration`

> Provided auction duration is zero or above the maximum.

**Returned by:** `create_auction` (message)

**Client guidance:** Input error: pass a duration within the allowed bounds (non-zero, at most the max); retry with a valid value.

### `TransferFailed`

> Underlying ERC20 transfer failed.

**Returned by:** `place_bid` (message), `transfer_winning_bid` (message), `withdraw_refund` (message)

**Client guidance:** The underlying ERC20 transfer failed: check token balance/allowance, then retry; report if balances look correct.

### `NoRefundAvailable`

> No refund balance is available to withdraw.

**Returned by:** `withdraw_refund` (message)

**Client guidance:** Nothing to withdraw; informational - do not retry.

### `BidAmountNotIncreased`

> A re-bid must strictly exceed the bidder's previous amount.

**Returned by:** `prepare_bid` (helper)

**Client guidance:** Input error: a re-bid must strictly exceed your previous bid; resubmit with a higher amount.

### `ArithmeticError`

> Arithmetic overflow or underflow.

**Returned by:** `create_auction` (message), `prepare_bid` (helper), `remove_active_auction` (helper)

**Client guidance:** Unexpected numeric error: report as a contract bug.
