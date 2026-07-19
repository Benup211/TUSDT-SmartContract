#![cfg_attr(not(feature = "std"), no_std, no_main)]
#![allow(clippy::enum_variant_names)]

pub use self::otc::{TusdtOtc, TusdtOtcRef};

#[ink::contract(env = tusdt_env::CustomEnvironment)]
mod otc {
    use ink::env::call::FromAccountId;
    use ink::storage::Mapping;
    use tusdt_erc20::TusdtErc20Ref;
    use tusdt_primitives::Ratio;

    /// Which side of the trade the maker is on.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub enum OrderSide {
        /// Maker provides collateral, wants alpha.
        Buy,
        /// Maker provides alpha, wants collateral.
        Sell,
    }

    /// The asset used as collateral in the trade.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub enum Collateral {
        /// TUSDT stablecoin (PSP22 token).
        Tusdt,
        /// Native chain token (TAO).
        Native,
    }

    /// Lifecycle state of an order.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub enum OrderStatus {
        /// Order is open and can be fulfilled or cancelled.
        Active,
        /// Order has been fulfilled and is closed.
        Fulfilled,
        /// Order was cancelled by the maker before fulfillment.
        Cancelled,
    }

    /// A single order in the OTC marketplace.
    #[derive(Debug, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct Order {
        /// Sequential order ID, assigned at creation.
        pub id: u64,
        /// Account that created the order.
        pub maker: AccountId,
        /// Subnet netuid for the alpha asset being traded.
        pub netuid: u16,
        /// Whether the maker is buying or selling alpha.
        pub side: OrderSide,
        /// The collateral asset the maker puts up.
        pub collateral: Collateral,
        /// The collateral asset the taker must provide.
        pub counter_collateral: Collateral,
        /// Units of counter-collateral per alpha unit, in basis points relative to
        /// `get_alpha_price(netuid)`.  10_000 = market price, 10_500 = 5 % above, etc.
        pub price_bps: u32,
        /// Amount of alpha (in RAO) to trade.
        pub alpha_amount: Balance,
        /// Current lifecycle status: Active, Fulfilled, or Cancelled.
        pub status: OrderStatus,
        /// Block timestamp (milliseconds) when the order was created.
        pub created_at: u64,
    }

    /// OTC swap marketplace storage: owner, hotkey, TUSDT token, fee rate,
    /// pause flag, order book, and per-netuid reserved-alpha tracking.
    #[ink(storage)]
    pub struct TusdtOtc {
        /// Contract owner; controls fee rate, pause, and excess-alpha claims.
        owner: AccountId,
        /// Hotkey used for alpha stake transfers via the chain extension.
        hotkey: AccountId,
        /// Reference to the TUSDT ERC20 token contract.
        token: TusdtErc20Ref,
        /// Fee in basis points (e.g. 30 = 0.3 %), deducted from the maker's side.
        fee_rate: Ratio,
        /// When `true` all trading operations are blocked.
        paused: bool,
        /// Map from order ID to full Order struct.
        orders: Mapping<u64, Order>,
        /// Auto-incrementing counter for the next order ID.
        next_order_id: u64,
        /// Per-netuid total alpha reserved by active Sell orders. Used to compute
        /// excess alpha (actual stake minus reserved) for `claim_excess_alpha`.
        netuid_total_reserved: Mapping<u16, Balance>,
    }

    // ── Events ────────────────────────────────────────────────────────

    /// Emitted when a new order is created.
    #[ink(event)]
    pub struct OrderCreated {
        #[ink(topic)]
        order_id: u64,
        #[ink(topic)]
        maker: AccountId,
        side: OrderSide,
    }

    /// Emitted when an active order is fulfilled by a taker.
    #[ink(event)]
    pub struct OrderFulfilled {
        #[ink(topic)]
        order_id: u64,
        #[ink(topic)]
        maker: AccountId,
        #[ink(topic)]
        taker: AccountId,
    }

    /// Emitted when a maker cancels their active order.
    #[ink(event)]
    pub struct OrderCancelled {
        #[ink(topic)]
        order_id: u64,
        #[ink(topic)]
        maker: AccountId,
    }

    /// Emitted when ownership is transferred to a new account.
    #[ink(event)]
    pub struct OwnerUpdated {
        previous: AccountId,
        new: AccountId,
    }

    /// Emitted when the fee rate is changed by the owner.
    #[ink(event)]
    pub struct FeeRateUpdated {
        previous_bps: u32,
        new_bps: u32,
    }

    /// Emitted when excess alpha is unstaked and native TAO is sent to a recipient.
    #[ink(event)]
    pub struct ExcessAlphaClaimed {
        #[ink(topic)]
        netuid: u16,
        excess_alpha: Balance,
        tao_received: Balance,
        #[ink(topic)]
        recipient: AccountId,
    }

    /// Emitted when the contract is paused by the owner.
    #[ink(event)]
    pub struct Paused {}

    /// Emitted when the contract is unpaused by the owner.
    #[ink(event)]
    pub struct Unpaused {}

    // ── Errors ────────────────────────────────────────────────────────

    /// Errors returned by the OTC marketplace contract.
    #[derive(Debug, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    pub enum Error {
        /// The caller is not the contract owner.
        NotOwner,
        /// The caller is not the order maker.
        NotMaker,
        /// The specified order ID does not exist.
        OrderNotFound,
        /// The order is not in Active status.
        OrderNotActive,
        /// A maker cannot fulfill their own order.
        CannotFulfillOwnOrder,
        /// The caller did not supply enough collateral (native transfers).
        InsufficientCollateral,
        /// The chain extension call failed (stake transfer, price read, etc.).
        ChainExtensionFailed,
        /// No alpha stake exists for the given hotkey/coldkey/netuid pair.
        NoAlphaStakeFound,
        /// The underlying token or native transfer call failed.
        TransferFailed,
        /// An arithmetic overflow or underflow occurred.
        ArithmeticError,
        /// The contract is paused and trading is disabled.
        ContractPaused,
        /// The `price_bps` value is zero, which is not allowed.
        InvalidPriceBps,
    }

    pub type Result<T> = core::result::Result<T, Error>;

    // ── Implementation ────────────────────────────────────────────────

    impl TusdtOtc {
        /// Initializes the OTC marketplace with an owner, hotkey, TUSDT token
        /// address, and a fee rate in basis points (e.g. 30 = 0.3%).
        ///
        /// The contract starts in the unpaused state. The `fee_rate_bps` is
        /// stored as an internal [`Ratio`] and is capped by the maximum
        /// basis-point value (10_000 = 100%).
        #[ink(constructor)]
        pub fn new(
            owner: AccountId,
            hotkey: AccountId,
            token_address: AccountId,
            fee_rate_bps: u32,
        ) -> Self {
            Self {
                owner,
                hotkey,
                token: TusdtErc20Ref::from_account_id(token_address),
                fee_rate: Ratio::from_basis_points(fee_rate_bps),
                paused: false,
                orders: Mapping::default(),
                next_order_id: 0,
                netuid_total_reserved: Mapping::default(),
            }
        }

        // ── Trading ────────────────────────────────────────────────

        /// Create an order.  For Sell orders, `alpha_amount` of the CALLER's alpha
        /// stake (held under this contract's hotkey) is pulled into the contract
        /// atomically via the caller-forwarded `caller_transfer_stake` chain extension
        /// — no prior deposit step is needed.  For Buy orders with `Tusdt` collateral
        /// the caller must have approved this contract to spend the required amount.
        ///
        /// `price_bps` is relative to the chain-extension alpha price:
        ///   10_000 = market,  10_500 = 5 % above,  9_500 = 5 % below.
        #[ink(message, payable)]
        pub fn create_order(
            &mut self,
            netuid: u16,
            side: OrderSide,
            collateral: Collateral,
            counter_collateral: Collateral,
            price_bps: u32,
            alpha_amount: Balance,
        ) -> Result<u64> {
            self.ensure_not_paused()?;
            if price_bps == 0 {
                return Err(Error::InvalidPriceBps);
            }
            if alpha_amount == 0 {
                return Err(Error::InsufficientCollateral);
            }

            let caller = self.env().caller();

            match side {
                OrderSide::Sell => {
                    // Atomic pull: transfer the maker's alpha into the contract's
                    // coldkey. The destination is always this contract.
                    self.env()
                        .extension()
                        .caller_transfer_stake(
                            self.env().account_id(),
                            self.hotkey,
                            netuid,
                            netuid,
                            alpha_amount,
                        )
                        .map_err(|_| Error::ChainExtensionFailed)?;
                    // Track this Sell order's alpha as reserved.
                    let current_reserved =
                        self.netuid_total_reserved.get(netuid).unwrap_or_default();
                    self.netuid_total_reserved.insert(
                        netuid,
                        &current_reserved
                            .checked_add(alpha_amount)
                            .ok_or(Error::ArithmeticError)?,
                    );
                },
                OrderSide::Buy => {
                    // Pull collateral from maker now so the order is backed.
                    self.pull_collateral(caller, collateral, alpha_amount, price_bps, netuid)?;
                },
            }

            let order_id = self.next_order_id;
            self.next_order_id = order_id.checked_add(1).ok_or(Error::ArithmeticError)?;

            let order = Order {
                id: order_id,
                maker: caller,
                netuid,
                side,
                collateral,
                counter_collateral,
                price_bps,
                alpha_amount,
                status: OrderStatus::Active,
                created_at: self.env().block_timestamp(),
            };
            self.orders.insert(order_id, &order);

            self.env().emit_event(OrderCreated { order_id, maker: caller, side });
            Ok(order_id)
        }

        /// Fulfill an active order.  Permissionless — anyone (except the maker)
        /// can call this.  Swaps the maker's provided asset for the taker's.
        #[ink(message, payable)]
        pub fn fulfill_order(&mut self, order_id: u64) -> Result<()> {
            self.ensure_not_paused()?;

            let mut order = self.orders.get(order_id).ok_or(Error::OrderNotFound)?;
            if order.status != OrderStatus::Active {
                return Err(Error::OrderNotActive);
            }
            let taker = self.env().caller();
            if taker == order.maker {
                return Err(Error::CannotFulfillOwnOrder);
            }

            // Compute settlement amounts.
            let alpha_price_rao = self
                .env()
                .extension()
                .get_alpha_price(order.netuid)
                .map_err(|_| Error::ChainExtensionFailed)?;

            // counter-collateral units per alpha = alpha_price_rao * price_bps / 10_000.
            // All in u128 to avoid overflow, then Ratio handles the division.
            let price_per_alpha = u128::from(alpha_price_rao)
                .checked_mul(u128::from(order.price_bps))
                .ok_or(Error::ArithmeticError)?
                .checked_div(10_000)
                .ok_or(Error::ArithmeticError)?;
            let counter_amount = u128::from(order.alpha_amount)
                .checked_mul(price_per_alpha)
                .ok_or(Error::ArithmeticError)?
                .checked_div(1_000_000_000)
                .ok_or(Error::ArithmeticError)?;
            let counter_amount =
                Balance::try_from(counter_amount).map_err(|_| Error::ArithmeticError)?;

            // Fee on the maker's side.
            let fee = self
                .fee_rate
                .checked_mul_value(counter_amount.into())
                .and_then(|f| Balance::try_from(f).ok())
                .ok_or(Error::ArithmeticError)?;

            // Execute transfers first, then mark fulfilled.
            match order.side {
                OrderSide::Sell => {
                    // Pull counter-collateral from taker before sending anything out.
                    self.pull_counter_collateral(taker, order.counter_collateral, counter_amount)?;
                    // Contract already holds maker's alpha (deposited at creation).
                    // Send alpha to taker; taker's counter-collateral goes to maker (minus fee).
                    self.transfer_alpha_to(order.netuid, taker, order.alpha_amount)?;
                    self.send_counter_collateral(
                        taker,
                        order.counter_collateral,
                        order.maker,
                        counter_amount,
                        fee,
                    )?;
                    // Release reserved alpha tracking.
                    let current_reserved =
                        self.netuid_total_reserved.get(order.netuid).unwrap_or_default();
                    self.netuid_total_reserved.insert(
                        order.netuid,
                        &current_reserved
                            .checked_sub(order.alpha_amount)
                            .ok_or(Error::ArithmeticError)?,
                    );
                },
                OrderSide::Buy => {
                    // Atomic pull: transfer the taker's alpha into the contract's
                    // coldkey via the caller-forwarded chain extension, then settle.
                    self.env()
                        .extension()
                        .caller_transfer_stake(
                            self.env().account_id(),
                            self.hotkey,
                            order.netuid,
                            order.netuid,
                            order.alpha_amount,
                        )
                        .map_err(|_| Error::ChainExtensionFailed)?;
                    // Send alpha from contract to maker (taker provided it).
                    self.transfer_alpha_to(order.netuid, order.maker, order.alpha_amount)?;
                    // Send maker's held collateral to taker.
                    self.send_counter_collateral(
                        self.env().account_id(),
                        order.collateral,
                        taker,
                        counter_amount,
                        fee,
                    )?;
                },
            }

            // All transfers succeeded — mark fulfilled.
            order.status = OrderStatus::Fulfilled;
            self.orders.insert(order_id, &order);

            self.env().emit_event(OrderFulfilled { order_id, maker: order.maker, taker });
            Ok(())
        }

        /// Cancel an active order.  Maker only.
        #[ink(message)]
        pub fn cancel_order(&mut self, order_id: u64) -> Result<()> {
            let mut order = self.orders.get(order_id).ok_or(Error::OrderNotFound)?;
            if self.env().caller() != order.maker {
                return Err(Error::NotMaker);
            }
            if order.status != OrderStatus::Active {
                return Err(Error::OrderNotActive);
            }

            order.status = OrderStatus::Cancelled;
            self.orders.insert(order_id, &order);

            // Return assets held by the contract.
            match order.side {
                OrderSide::Buy => {
                    // Return locked collateral to maker.
                    let alpha_price_rao = self
                        .env()
                        .extension()
                        .get_alpha_price(order.netuid)
                        .map_err(|_| Error::ChainExtensionFailed)?;
                    let price_per_alpha = u128::from(alpha_price_rao)
                        .checked_mul(u128::from(order.price_bps))
                        .ok_or(Error::ArithmeticError)?
                        .checked_div(10_000)
                        .ok_or(Error::ArithmeticError)?;
                    let locked = u128::from(order.alpha_amount)
                        .checked_mul(price_per_alpha)
                        .ok_or(Error::ArithmeticError)?
                        .checked_div(1_000_000_000)
                        .ok_or(Error::ArithmeticError)?;
                    let locked = Balance::try_from(locked).map_err(|_| Error::ArithmeticError)?;
                    self.return_collateral(order.maker, order.collateral, locked)?;
                },
                OrderSide::Sell => {
                    // Release reserved alpha tracking before returning stake.
                    let current_reserved =
                        self.netuid_total_reserved.get(order.netuid).unwrap_or_default();
                    self.netuid_total_reserved.insert(
                        order.netuid,
                        &current_reserved
                            .checked_sub(order.alpha_amount)
                            .ok_or(Error::ArithmeticError)?,
                    );
                    // Return alpha stake back to maker.
                    self.transfer_alpha_to(order.netuid, order.maker, order.alpha_amount)?;
                },
            }

            self.env().emit_event(OrderCancelled { order_id, maker: order.maker });
            Ok(())
        }

        // ── Owner ────────────────────────────────────────────────────

        /// Transfers contract ownership to a new account. Callable only by the current owner.
        #[ink(message)]
        pub fn update_owner(&mut self, new_owner: AccountId) -> Result<()> {
            self.ensure_owner()?;
            let previous = self.owner;
            self.owner = new_owner;
            self.env().emit_event(OwnerUpdated { previous, new: new_owner });
            Ok(())
        }

        /// Updates the protocol fee rate. Callable only by the owner.
        /// `fee_rate_bps` is in basis points (e.g. 30 = 0.3%).
        #[ink(message)]
        pub fn update_fee_rate(&mut self, fee_rate_bps: u32) -> Result<()> {
            self.ensure_owner()?;
            let previous_bps = self.fee_rate.to_basis_points().unwrap_or(0);
            self.fee_rate = Ratio::from_basis_points(fee_rate_bps);
            self.env().emit_event(FeeRateUpdated { previous_bps, new_bps: fee_rate_bps });
            Ok(())
        }

        /// Pauses the contract, blocking all trading operations. Owner only.
        #[ink(message)]
        pub fn pause(&mut self) -> Result<()> {
            self.ensure_owner()?;
            self.paused = true;
            self.env().emit_event(Paused {});
            Ok(())
        }

        /// Unpauses the contract, re-enabling trading operations. Owner only.
        #[ink(message)]
        pub fn unpause(&mut self) -> Result<()> {
            self.ensure_owner()?;
            self.paused = false;
            self.env().emit_event(Unpaused {});
            Ok(())
        }

        /// Claims excess alpha staking rewards on a subnet by unstaking them to native
        /// TAO and transferring the TAO to the specified recipient. Owner only.
        ///
        /// Excess = actual contract stake on the subnet minus total alpha reserved by
        /// active Sell orders.  If there is no excess, this is a no-op (returns Ok).
        #[ink(message)]
        pub fn claim_excess_alpha(&mut self, netuid: u16, recipient: AccountId) -> Result<()> {
            self.ensure_owner()?;

            let total_reserved = self.netuid_total_reserved.get(netuid).unwrap_or_default();
            let current_stake = self.get_contract_stake(netuid)?;

            if current_stake <= total_reserved {
                return Ok(());
            }

            let excess = current_stake.checked_sub(total_reserved).ok_or(Error::ArithmeticError)?;

            let balance_before = self.env().balance();

            self.env()
                .extension()
                .remove_stake(self.hotkey, netuid, excess)
                .map_err(|_| Error::ChainExtensionFailed)?;

            let balance_after = self.env().balance();
            let tao_received =
                balance_after.checked_sub(balance_before).ok_or(Error::ArithmeticError)?;

            if tao_received > 0 {
                self.env().transfer(recipient, tao_received).map_err(|_| Error::TransferFailed)?;
            }

            self.env().emit_event(ExcessAlphaClaimed {
                netuid,
                excess_alpha: excess,
                tao_received,
                recipient,
            });

            Ok(())
        }

        // ── Queries ──────────────────────────────────────────────────

        /// Returns the full `Order` struct for a given order ID, or `None` if not found.
        #[ink(message)]
        pub fn get_order(&self, order_id: u64) -> Option<Order> {
            self.orders.get(order_id)
        }

        /// Returns the next available order ID (auto-increment counter).
        #[ink(message)]
        pub fn get_next_order_id(&self) -> u64 {
            self.next_order_id
        }

        /// Returns the contract owner account ID.
        #[ink(message)]
        pub fn owner(&self) -> AccountId {
            self.owner
        }

        /// Returns the hotkey used for alpha stake operations.
        #[ink(message)]
        pub fn hotkey(&self) -> AccountId {
            self.hotkey
        }

        /// Returns `true` if the contract is currently paused.
        #[ink(message)]
        pub fn is_paused(&self) -> bool {
            self.paused
        }

        /// Returns the current fee rate in basis points (e.g. 30 = 0.3%).
        #[ink(message)]
        pub fn fee_rate_bps(&self) -> u32 {
            self.fee_rate.to_basis_points().unwrap_or(0)
        }

        /// Returns the total amount of alpha (in RAO) reserved by active Sell
        /// orders on a subnet.
        #[ink(message)]
        pub fn get_reserved_alpha(&self, netuid: u16) -> Balance {
            self.netuid_total_reserved.get(netuid).unwrap_or_default()
        }

        // ── Internals ────────────────────────────────────────────────

        /// Reverts with `NotOwner` if the caller is not the contract owner.
        fn ensure_owner(&self) -> Result<()> {
            if self.env().caller() != self.owner {
                return Err(Error::NotOwner);
            }
            Ok(())
        }

        /// Reverts with `ContractPaused` if the contract is currently paused.
        fn ensure_not_paused(&self) -> Result<()> {
            if self.paused {
                return Err(Error::ContractPaused);
            }
            Ok(())
        }

        /// Queries the chain extension for the contract's currently-available
        /// (total minus locked) stake on a subnet. Returns `NoAlphaStakeFound`
        /// if no stake entry exists.
        fn get_contract_stake(&mut self, netuid: u16) -> Result<Balance> {
            let info = self
                .env()
                .extension()
                .get_stake_info_for_hotkey_coldkey_netuid(
                    self.hotkey,
                    self.env().account_id(),
                    netuid,
                )
                .map_err(|_| Error::ChainExtensionFailed)?
                .ok_or(Error::NoAlphaStakeFound)?;
            let stake: u64 = info.stake.into();
            let locked: u64 = info.locked.into();
            stake.checked_sub(locked).ok_or(Error::ArithmeticError)
        }

        /// Pulls required collateral from the maker into the contract's custody.
        /// Used for Buy orders: TUSDT via `transfer_from`, Native via
        /// `transferred_value`.
        fn pull_collateral(
            &mut self,
            maker: AccountId,
            collateral: Collateral,
            alpha_amount: Balance,
            price_bps: u32,
            netuid: u16,
        ) -> Result<()> {
            let alpha_price_rao = self
                .env()
                .extension()
                .get_alpha_price(netuid)
                .map_err(|_| Error::ChainExtensionFailed)?;
            let price_per_alpha = u128::from(alpha_price_rao)
                .checked_mul(u128::from(price_bps))
                .ok_or(Error::ArithmeticError)?
                .checked_div(10_000)
                .ok_or(Error::ArithmeticError)?;
            let required = u128::from(alpha_amount)
                .checked_mul(price_per_alpha)
                .ok_or(Error::ArithmeticError)?
                .checked_div(1_000_000_000)
                .ok_or(Error::ArithmeticError)?;
            let required = Balance::try_from(required).map_err(|_| Error::ArithmeticError)?;

            match collateral {
                Collateral::Tusdt => {
                    self.token
                        .transfer_from(maker, self.env().account_id(), required)
                        .map_err(|_| Error::TransferFailed)?;
                },
                Collateral::Native => {
                    let sent = self.env().transferred_value();
                    if sent < required {
                        return Err(Error::InsufficientCollateral);
                    }
                },
            }
            Ok(())
        }

        /// Pulls counter-collateral from the taker at fulfillment time. Used
        /// for Sell orders where the taker provides collateral to receive the
        /// maker's alpha. TUSDT via `transfer_from`, Native via `transferred_value`.
        fn pull_counter_collateral(
            &mut self,
            taker: AccountId,
            kind: Collateral,
            amount: Balance,
        ) -> Result<()> {
            match kind {
                Collateral::Tusdt => {
                    self.token
                        .transfer_from(taker, self.env().account_id(), amount)
                        .map_err(|_| Error::TransferFailed)?;
                },
                Collateral::Native => {
                    let sent = self.env().transferred_value();
                    if sent < amount {
                        return Err(Error::InsufficientCollateral);
                    }
                },
            }
            Ok(())
        }

        /// Transfers alpha stake from the contract's coldkey to a recipient
        /// via the chain extension `transfer_stake` function.
        fn transfer_alpha_to(&mut self, netuid: u16, to: AccountId, amount: Balance) -> Result<()> {
            self.env()
                .extension()
                .transfer_stake(to, self.hotkey, netuid, netuid, amount)
                .map_err(|_| Error::ChainExtensionFailed)
        }

        /// Sends counter-collateral to the recipient after deducting the
        /// protocol fee, which is forwarded to the owner. Supports both
        /// TUSDT and Native token kinds.
        fn send_counter_collateral(
            &mut self,
            _payer: AccountId,
            kind: Collateral,
            recipient: AccountId,
            amount: Balance,
            fee: Balance,
        ) -> Result<()> {
            let net = amount.checked_sub(fee).ok_or(Error::ArithmeticError)?;
            match kind {
                Collateral::Tusdt => {
                    if net > 0 {
                        self.token.transfer(recipient, net).map_err(|_| Error::TransferFailed)?;
                    }
                    if fee > 0 {
                        self.token.transfer(self.owner, fee).map_err(|_| Error::TransferFailed)?;
                    }
                },
                Collateral::Native => {
                    if net > 0 && self.env().transfer(recipient, net).is_err() {
                        return Err(Error::TransferFailed);
                    }
                    if fee > 0 && self.env().transfer(self.owner, fee).is_err() {
                        return Err(Error::TransferFailed);
                    }
                },
            }
            Ok(())
        }

        /// Returns locked collateral to the maker after a Buy-order cancellation.
        fn return_collateral(
            &mut self,
            recipient: AccountId,
            kind: Collateral,
            amount: Balance,
        ) -> Result<()> {
            match kind {
                Collateral::Tusdt => {
                    self.token.transfer(recipient, amount).map_err(|_| Error::TransferFailed)?;
                },
                Collateral::Native => {
                    if self.env().transfer(recipient, amount).is_err() {
                        return Err(Error::TransferFailed);
                    }
                },
            }
            Ok(())
        }
    }

    #[cfg(test)]
    impl TusdtOtc {
        /// Test-only constructor that sets up a minimal contract state for
        /// unit testing. Uses default accounts (owner as specified, bob as
        /// hotkey, charlie as token address).
        pub(crate) fn new_for_test(owner: AccountId) -> Self {
            use ink::env::call::FromAccountId;
            let accounts = ink::env::test::default_accounts::<tusdt_env::CustomEnvironment>();
            Self {
                owner,
                hotkey: accounts.bob,
                token: TusdtErc20Ref::from_account_id(accounts.charlie),
                fee_rate: Ratio::from_basis_points(30),
                paused: false,
                orders: Mapping::default(),
                next_order_id: 0,
                netuid_total_reserved: Mapping::default(),
            }
        }
    }
}

#[cfg(test)]
mod tests;
