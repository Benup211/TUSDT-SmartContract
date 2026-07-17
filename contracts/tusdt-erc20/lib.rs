#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use self::tusdt::{TusdtErc20, TusdtErc20Ref};

#[ink::contract(env = tusdt_env::CustomEnvironment)]
mod tusdt {
    use ink::storage::Mapping;

    /// Storage for the tUSDT ERC20-style stablecoin: controller, minters, supply, balances, and allowances.
    #[ink(storage)]
    pub struct TusdtErc20 {
        controller: AccountId,
        minters: Mapping<AccountId, ()>,
        total_supply: Balance,
        balances: Mapping<AccountId, Balance>,
        allowances: Mapping<(AccountId, AccountId), Balance>,
    }

    /// Emitted on token movement; `from = None` denotes a mint, `to = None` denotes a burn.
    #[ink(event)]
    pub struct Transfer {
        #[ink(topic)]
        from: Option<AccountId>,
        #[ink(topic)]
        to: Option<AccountId>,
        value: Balance,
    }

    /// Emitted whenever an owner sets or updates a spender's allowance.
    #[ink(event)]
    pub struct Approval {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        spender: AccountId,
        value: Balance,
    }

    /// Errors returned by the tUSDT token contract.
    #[derive(Debug, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    pub enum Error {
        /// Sender's balance is below the requested amount.
        InsufficientBalance,
        /// Caller's allowance from the owner is below the requested amount.
        InsufficientAllowance,
        /// Caller is not an authorized minter account.
        NotMinter,
        /// Caller is not the configured controller account.
        NotController,
        /// An arithmetic overflow or underflow occurred.
        ArithmeticError,
    }

    pub type Result<T> = core::result::Result<T, Error>;

    impl TusdtErc20 {
        /// Initializes the token contract with the specified controller account. The controller
        /// is also registered as the initial minter.
        #[ink(constructor)]
        pub fn new(controller: AccountId) -> Self {
            let mut minters = Mapping::default();
            minters.insert(controller, &());
            Self {
                controller,
                minters,
                total_supply: 0,
                balances: Mapping::default(),
                allowances: Default::default(),
            }
        }

        /// Returns the controller account ID.
        #[ink(message)]
        pub fn controller(&self) -> AccountId {
            self.controller
        }

        /// Transfers the controller role to a new account. Callable only by the current
        /// controller. Removes the old controller from minters and adds the new one.
        #[ink(message)]
        pub fn set_controller(&mut self, new_controller: AccountId) -> Result<()> {
            let old_controller = self.controller;
            self.ensure_controller()?;
            self.minters.remove(old_controller);
            self.minters.insert(new_controller, &());
            self.controller = new_controller;
            Ok(())
        }

        /// Adds an account to the authorized minters set. Only callable by the controller.
        #[ink(message)]
        pub fn add_minter(&mut self, minter: AccountId) -> Result<()> {
            self.ensure_controller()?;
            self.minters.insert(minter, &());
            Ok(())
        }

        /// Removes an account from the authorized minters set. Only callable by the controller.
        #[ink(message)]
        pub fn remove_minter(&mut self, minter: AccountId) -> Result<()> {
            self.ensure_controller()?;
            self.minters.remove(minter);
            Ok(())
        }

        /// Returns `true` if the given account is an authorized minter.
        #[ink(message)]
        pub fn is_minter(&self, account: AccountId) -> bool {
            self.minters.get(account).is_some()
        }

        /// Returns the total supply of tokens in circulation.
        #[ink(message)]
        pub fn total_supply(&self) -> Balance {
            self.total_supply
        }

        /// Returns the token balance of an account.
        #[ink(message)]
        pub fn balance_of(&self, owner: AccountId) -> Balance {
            self.balance_of_impl(&owner)
        }

        /// Internal balance lookup; returns 0 for accounts with no entry.
        #[inline]
        fn balance_of_impl(&self, owner: &AccountId) -> Balance {
            self.balances.get(owner).unwrap_or_default()
        }

        /// Returns the amount of tokens that a spender is allowed to transfer from an owner's account.
        #[ink(message)]
        pub fn allowance(&self, owner: AccountId, spender: AccountId) -> Balance {
            self.allowance_impl(&owner, &spender)
        }

        /// Internal allowance lookup; returns 0 when no allowance is set.
        #[inline]
        fn allowance_impl(&self, owner: &AccountId, spender: &AccountId) -> Balance {
            self.allowances.get((owner, spender)).unwrap_or_default()
        }

        /// Writes the allowance entry and emits an `Approval` event.
        #[inline]
        fn set_allowance(&mut self, owner: AccountId, spender: AccountId, value: Balance) {
            self.allowances.insert((&owner, &spender), &value);
            self.env().emit_event(Approval {
                owner,
                spender,
                value,
            });
        }

        /// Reverts with `NotController` if the caller is not the configured controller.
        #[inline]
        fn ensure_controller(&self) -> Result<()> {
            if self.env().caller() != self.controller {
                return Err(Error::NotController);
            }
            Ok(())
        }

        /// Reverts with `NotMinter` if the caller is not in the minters set.
        #[inline]
        fn ensure_minter(&self) -> Result<()> {
            if self.minters.get(self.env().caller()).is_none() {
                return Err(Error::NotMinter);
            }
            Ok(())
        }

        /// Transfers tokens from the caller to a recipient account.
        #[ink(message)]
        pub fn transfer(&mut self, to: AccountId, value: Balance) -> Result<()> {
            let from = self.env().caller();
            self.transfer_from_to(&from, &to, value)
        }

        /// Mints new tokens and adds them to an account's balance; only callable by authorized minters.
        #[ink(message)]
        pub fn mint(&mut self, to: AccountId, value: Balance) -> Result<()> {
            self.ensure_minter()?;
            let to_balance = self.balance_of_impl(&to);

            self.total_supply = self
                .total_supply
                .checked_add(value)
                .ok_or(Error::ArithmeticError)?;
            self.balances.insert(
                to,
                &to_balance
                    .checked_add(value)
                    .ok_or(Error::ArithmeticError)?,
            );

            self.env().emit_event(Transfer {
                from: None,
                to: Some(to),
                value,
            });
            Ok(())
        }

        /// Burns tokens from an account, reducing the total supply; only callable by authorized minters.
        #[ink(message)]
        pub fn burn(&mut self, from: AccountId, value: Balance) -> Result<()> {
            self.ensure_minter()?;
            let from_balance = self.balance_of_impl(&from);
            if from_balance < value {
                return Err(Error::InsufficientBalance);
            }
            // We checked that from_balance >= value
            #[allow(clippy::arithmetic_side_effects)]
            self.balances.insert(from, &(from_balance - value));

            self.total_supply = self
                .total_supply
                .checked_sub(value)
                .ok_or(Error::ArithmeticError)?;

            self.env().emit_event(Transfer {
                from: Some(from),
                to: None,
                value,
            });
            Ok(())
        }

        /// Approves a spender to transfer up to a specified amount of tokens on behalf of the caller.
        #[ink(message)]
        pub fn approve(&mut self, spender: AccountId, value: Balance) -> Result<()> {
            let owner = self.env().caller();
            self.set_allowance(owner, spender, value);
            Ok(())
        }

        /// Increases a spender's allowance by a specified amount.
        #[ink(message)]
        pub fn increase_allowance(
            &mut self,
            spender: AccountId,
            delta_value: Balance,
        ) -> Result<()> {
            let owner = self.env().caller();
            let allowance = self.allowance_impl(&owner, &spender);
            let updated_allowance = allowance.saturating_add(delta_value);
            self.set_allowance(owner, spender, updated_allowance);
            Ok(())
        }

        /// Decreases a spender's allowance by a specified amount.
        #[ink(message)]
        pub fn decrease_allowance(
            &mut self,
            spender: AccountId,
            delta_value: Balance,
        ) -> Result<()> {
            let owner = self.env().caller();
            let allowance = self.allowance_impl(&owner, &spender);
            let updated_allowance = allowance
                .checked_sub(delta_value)
                .ok_or(Error::InsufficientAllowance)?;
            self.set_allowance(owner, spender, updated_allowance);
            Ok(())
        }

        /// Transfers tokens on behalf of an owner account to a recipient, using the caller's allowance.
        #[ink(message)]
        pub fn transfer_from(
            &mut self,
            from: AccountId,
            to: AccountId,
            value: Balance,
        ) -> Result<()> {
            let caller = self.env().caller();
            let allowance = self.allowance_impl(&from, &caller);
            if allowance < value {
                return Err(Error::InsufficientAllowance);
            }
            self.transfer_from_to(&from, &to, value)?;
            // We checked that allowance >= value
            #[allow(clippy::arithmetic_side_effects)]
            self.allowances
                .insert((&from, &caller), &(allowance - value));
            Ok(())
        }

        /// Core balance-moving routine shared by `transfer` and `transfer_from`; emits a `Transfer` event.
        fn transfer_from_to(
            &mut self,
            from: &AccountId,
            to: &AccountId,
            value: Balance,
        ) -> Result<()> {
            let from_balance = self.balance_of_impl(from);
            if from_balance < value {
                return Err(Error::InsufficientBalance);
            }
            // We checked that from_balance >= value
            #[allow(clippy::arithmetic_side_effects)]
            self.balances.insert(from, &(from_balance - value));
            let to_balance = self.balance_of_impl(to);
            self.balances.insert(
                to,
                &to_balance
                    .checked_add(value)
                    .ok_or(Error::ArithmeticError)?,
            );
            self.env().emit_event(Transfer {
                from: Some(*from),
                to: Some(*to),
                value,
            });
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
