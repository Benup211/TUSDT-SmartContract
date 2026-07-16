#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use self::otc::{TusdtOtc, TusdtOtcRef};

#[ink::contract(env = tusdt_env::CustomEnvironment)]
mod otc {
    use ink::env::call::FromAccountId;
    use ink::storage::Mapping;
    use tusdt_erc20::TusdtErc20Ref;
    use tusdt_primitives::Ratio;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub enum OrderSide {
        /// Maker provides collateral, wants alpha.
        Buy,
        /// Maker provides alpha, wants collateral.
        Sell,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub enum Collateral {
        Tusdt,
        Native,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub enum OrderStatus {
        Active,
        Fulfilled,
        Cancelled,
    }

    #[derive(Debug, Clone)]
    #[ink::scale_derive(Decode, Encode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub struct Order {
        pub id: u64,
        pub maker: AccountId,
        pub netuid: u16,
        pub side: OrderSide,
        pub collateral: Collateral,
        pub counter_collateral: Collateral,
        /// Units of counter-collateral per alpha unit, in basis points relative to
        /// `get_alpha_price(netuid)`.  10_000 = market price, 10_500 = 5 % above, etc.
        pub price_bps: u32,
        pub alpha_amount: Balance,
        pub status: OrderStatus,
        pub created_at: u64,
    }

    #[ink(storage)]
    pub struct TusdtOtc {
        owner: AccountId,
        hotkey: AccountId,
        token: TusdtErc20Ref,
        /// Fee in basis points (e.g. 30 = 0.3 %), deducted from the maker's side.
        fee_rate: Ratio,
        paused: bool,
        orders: Mapping<u64, Order>,
        next_order_id: u64,
        /// Two-step deposit intent: (caller, netuid) → alpha amount.
        pending_deposits: Mapping<(AccountId, u16), Balance>,
        /// Per-netuid total alpha reserved by active Sell orders. Used to compute
        /// excess alpha (actual stake minus reserved) for `claim_excess_alpha`.
        netuid_total_reserved: Mapping<u16, Balance>,
    }

    // ── Events ────────────────────────────────────────────────────────

    #[ink(event)]
    pub struct OrderCreated {
        #[ink(topic)]
        order_id: u64,
        #[ink(topic)]
        maker: AccountId,
        side: OrderSide,
    }

    #[ink(event)]
    pub struct OrderFulfilled {
        #[ink(topic)]
        order_id: u64,
        #[ink(topic)]
        maker: AccountId,
        #[ink(topic)]
        taker: AccountId,
    }

    #[ink(event)]
    pub struct OrderCancelled {
        #[ink(topic)]
        order_id: u64,
        #[ink(topic)]
        maker: AccountId,
    }

    #[ink(event)]
    pub struct OwnerUpdated { previous: AccountId, new: AccountId }

    #[ink(event)]
    pub struct FeeRateUpdated { previous_bps: u32, new_bps: u32 }

    #[ink(event)]
    pub struct ExcessAlphaClaimed {
        #[ink(topic)]
        netuid: u16,
        excess_alpha: Balance,
        tao_received: Balance,
        #[ink(topic)]
        recipient: AccountId,
    }

    #[ink(event)]
    pub struct Paused {}

    #[ink(event)]
    pub struct Unpaused {}

    // ── Errors ────────────────────────────────────────────────────────

    #[derive(Debug, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    pub enum Error {
        NotOwner,
        NotMaker,
        OrderNotFound,
        OrderNotActive,
        CannotFulfillOwnOrder,
        InsufficientCollateral,
        ChainExtensionFailed,
        NoAlphaStakeFound,
        TransferFailed,
        ArithmeticError,
        ContractPaused,
        InvalidPriceBps,
    }

    pub type Result<T> = core::result::Result<T, Error>;

    // ── Implementation ────────────────────────────────────────────────

    impl TusdtOtc {
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
                pending_deposits: Mapping::default(),
                netuid_total_reserved: Mapping::default(),
            }
        }

        // ── Trading ────────────────────────────────────────────────

        /// Step 1 of two-step alpha deposit: register intent to sell alpha.
        #[ink(message)]
        pub fn deposit_alpha(&mut self, amount: Balance, netuid: u16) -> Result<()> {
            self.ensure_not_paused()?;
            if amount == 0 {
                return Err(Error::InsufficientCollateral);
            }
            let caller = self.env().caller();
            if self.pending_deposits.get((caller, netuid)).is_some() {
                return Err(Error::InsufficientCollateral);
            }
            self.pending_deposits.insert((caller, netuid), &amount);
            Ok(())
        }

        /// Create an order.  For Sell orders the caller must have first called
        /// `deposit_alpha` and transferred alpha stake to this contract.  For
        /// Buy orders with `Tusdt` collateral the caller must have approved this
        /// contract to spend the required amount.
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
                    // Two-step deposit: caller must have registered intent with enough alpha.
                    let deposited = self
                        .pending_deposits
                        .take((caller, netuid))
                        .ok_or(Error::InsufficientCollateral)?;
                    if deposited < alpha_amount {
                        return Err(Error::InsufficientCollateral);
                    }
                    // Stake verification: contract actually holds enough alpha.
                    let available = self.get_contract_stake(netuid)?;
                    if available < alpha_amount {
                        return Err(Error::InsufficientCollateral);
                    }
                    // If the user deposited more than needed, re-register the remainder.
                    let remainder = deposited.checked_sub(alpha_amount)
                        .ok_or(Error::ArithmeticError)?;
                    if remainder > 0 {
                        self.pending_deposits.insert((caller, netuid), &remainder);
                    }
                    // Track this Sell order's alpha as reserved.
                    let current_reserved = self.netuid_total_reserved.get(netuid).unwrap_or_default();
                    self.netuid_total_reserved.insert(
                        netuid,
                        &current_reserved
                            .checked_add(alpha_amount)
                            .ok_or(Error::ArithmeticError)?,
                    );
                }
                OrderSide::Buy => {
                    // Pull collateral from maker now so the order is backed.
                    self.pull_collateral(caller, collateral, alpha_amount, price_bps, netuid)?;
                }
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
                    self.send_counter_collateral(taker, order.counter_collateral, order.maker, counter_amount, fee)?;
                    // Release reserved alpha tracking.
                    let current_reserved = self.netuid_total_reserved.get(order.netuid).unwrap_or_default();
                    self.netuid_total_reserved.insert(
                        order.netuid,
                        &current_reserved
                            .checked_sub(order.alpha_amount)
                            .ok_or(Error::ArithmeticError)?,
                    );
                }
                OrderSide::Buy => {
                    // Taker must have called deposit_alpha + transferred stake to
                    // the contract before fulfilling.  Verify the taker's deposit.
                    let taker_deposit = self
                        .pending_deposits
                        .get((taker, order.netuid))
                        .ok_or(Error::InsufficientCollateral)?;
                    if taker_deposit < order.alpha_amount {
                        return Err(Error::InsufficientCollateral);
                    }
                    // Consume the taker's deposit and verify actual stake.
                    self.pending_deposits.remove((taker, order.netuid));
                    let _ = self.get_contract_stake(order.netuid)?;
                    // Send alpha from contract to maker (taker provided it).
                    self.transfer_alpha_to(order.netuid, order.maker, order.alpha_amount)?;
                    // Send maker's held collateral to taker.
                    self.send_counter_collateral(self.env().account_id(), order.collateral, taker, counter_amount, fee)?;
                }
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
                    let locked =
                        Balance::try_from(locked).map_err(|_| Error::ArithmeticError)?;
                    self.return_collateral(order.maker, order.collateral, locked)?;
                }
                OrderSide::Sell => {
                    // Release reserved alpha tracking before returning stake.
                    let current_reserved = self.netuid_total_reserved.get(order.netuid).unwrap_or_default();
                    self.netuid_total_reserved.insert(
                        order.netuid,
                        &current_reserved
                            .checked_sub(order.alpha_amount)
                            .ok_or(Error::ArithmeticError)?,
                    );
                    // Return alpha stake back to maker.
                    self.transfer_alpha_to(order.netuid, order.maker, order.alpha_amount)?;
                }
            }

            self.env().emit_event(OrderCancelled { order_id, maker: order.maker });
            Ok(())
        }

        // ── Owner ────────────────────────────────────────────────────

        #[ink(message)]
        pub fn update_owner(&mut self, new_owner: AccountId) -> Result<()> {
            self.ensure_owner()?;
            let previous = self.owner;
            self.owner = new_owner;
            self.env().emit_event(OwnerUpdated { previous, new: new_owner });
            Ok(())
        }

        #[ink(message)]
        pub fn update_fee_rate(&mut self, fee_rate_bps: u32) -> Result<()> {
            self.ensure_owner()?;
            let previous_bps = self.fee_rate.to_basis_points().unwrap_or(0);
            self.fee_rate = Ratio::from_basis_points(fee_rate_bps);
            self.env().emit_event(FeeRateUpdated { previous_bps, new_bps: fee_rate_bps });
            Ok(())
        }

        #[ink(message)]
        pub fn pause(&mut self) -> Result<()> {
            self.ensure_owner()?;
            self.paused = true;
            self.env().emit_event(Paused {});
            Ok(())
        }

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

            let excess = current_stake
                .checked_sub(total_reserved)
                .ok_or(Error::ArithmeticError)?;

            let balance_before = self.env().balance();

            self.env()
                .extension()
                .remove_stake(self.hotkey, netuid, excess)
                .map_err(|_| Error::ChainExtensionFailed)?;

            let balance_after = self.env().balance();
            let tao_received = balance_after
                .checked_sub(balance_before)
                .ok_or(Error::ArithmeticError)?;

            if tao_received > 0 {
                self.env()
                    .transfer(recipient, tao_received)
                    .map_err(|_| Error::TransferFailed)?;
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

        #[ink(message)]
        pub fn get_order(&self, order_id: u64) -> Option<Order> {
            self.orders.get(order_id)
        }

        #[ink(message)]
        pub fn get_next_order_id(&self) -> u64 {
            self.next_order_id
        }

        #[ink(message)]
        pub fn owner(&self) -> AccountId { self.owner }

        #[ink(message)]
        pub fn hotkey(&self) -> AccountId { self.hotkey }

        #[ink(message)]
        pub fn is_paused(&self) -> bool { self.paused }

        #[ink(message)]
        pub fn fee_rate_bps(&self) -> u32 {
            self.fee_rate.to_basis_points().unwrap_or(0)
        }

        #[ink(message)]
        pub fn get_reserved_alpha(&self, netuid: u16) -> Balance {
            self.netuid_total_reserved.get(netuid).unwrap_or_default()
        }

        // ── Internals ────────────────────────────────────────────────

        fn ensure_owner(&self) -> Result<()> {
            if self.env().caller() != self.owner {
                return Err(Error::NotOwner);
            }
            Ok(())
        }

        fn ensure_not_paused(&self) -> Result<()> {
            if self.paused {
                return Err(Error::ContractPaused);
            }
            Ok(())
        }

        fn get_contract_stake(&mut self, netuid: u16) -> Result<Balance> {
            let info = self
                .env()
                .extension()
                .get_stake_info_for_hotkey_coldkey_netuid(
                    self.hotkey, self.env().account_id(), netuid,
                )
                .map_err(|_| Error::ChainExtensionFailed)?
                .ok_or(Error::NoAlphaStakeFound)?;
            let stake: u64 = info.stake.into();
            let locked: u64 = info.locked.into();
            stake.checked_sub(locked).ok_or(Error::ArithmeticError)
        }

        /// Pull collateral from maker into contract custody (Buy orders).
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
            let required =
                Balance::try_from(required).map_err(|_| Error::ArithmeticError)?;

            match collateral {
                Collateral::Tusdt => {
                    self.token
                        .transfer_from(maker, self.env().account_id(), required)
                        .map_err(|_| Error::TransferFailed)?;
                }
                Collateral::Native => {
                    let sent = self.env().transferred_value();
                    if sent < required {
                        return Err(Error::InsufficientCollateral);
                    }
                }
            }
            Ok(())
        }

        /// Pull counter-collateral from taker at fulfillment time.  Called for Sell
        /// orders where the taker provides collateral to receive the maker's alpha.
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
                }
                Collateral::Native => {
                    let sent = self.env().transferred_value();
                    if sent < amount {
                        return Err(Error::InsufficientCollateral);
                    }
                }
            }
            Ok(())
        }

        fn transfer_alpha_to(
            &mut self,
            netuid: u16,
            to: AccountId,
            amount: Balance,
        ) -> Result<()> {
            self.env()
                .extension()
                .transfer_stake(to, self.hotkey, netuid, netuid, amount)
                .map_err(|_| Error::ChainExtensionFailed)
        }

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
                        self.token
                            .transfer(recipient, net)
                            .map_err(|_| Error::TransferFailed)?;
                    }
                    if fee > 0 {
                        self.token
                            .transfer(self.owner, fee)
                            .map_err(|_| Error::TransferFailed)?;
                    }
                }
                Collateral::Native => {
                    if net > 0 && self.env().transfer(recipient, net).is_err() {
                        return Err(Error::TransferFailed);
                    }
                    if fee > 0 && self.env().transfer(self.owner, fee).is_err() {
                        return Err(Error::TransferFailed);
                    }
                }
            }
            Ok(())
        }

        fn return_collateral(
            &mut self,
            recipient: AccountId,
            kind: Collateral,
            amount: Balance,
        ) -> Result<()> {
            match kind {
                Collateral::Tusdt => {
                    self.token
                        .transfer(recipient, amount)
                        .map_err(|_| Error::TransferFailed)?;
                }
                Collateral::Native => {
                    if self.env().transfer(recipient, amount).is_err() {
                        return Err(Error::TransferFailed);
                    }
                }
            }
            Ok(())
        }
    }

    #[cfg(test)]
    impl TusdtOtc {
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
                pending_deposits: Mapping::default(),
                netuid_total_reserved: Mapping::default(),
            }
        }
    }
}

#[cfg(test)]
mod tests;
