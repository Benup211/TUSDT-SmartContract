#![cfg_attr(not(feature = "std"), no_std, no_main)]
#![allow(clippy::enum_variant_names)]

pub use self::treasury::{
    Fund, FundReleased, FundsDistributed, GovernanceUpdated, TokenKind, TusdtTreasury,
    TusdtTreasuryRef,
};

#[ink::contract(env = tusdt_env::CustomEnvironment)]
mod treasury {
    use ink::env::call::FromAccountId;
    use ink::storage::Mapping;
    use tusdt_erc20::TusdtErc20Ref;

    /// Distribution share for the Emergency fund, in basis points (5,000 = 50%).
    #[allow(dead_code)]
    pub(crate) const EMERGENCY_BPS: u32 = 5000;
    /// Distribution share for the Operation fund, in basis points (3,000 = 30%).
    pub(crate) const OPERATION_BPS: u32 = 3000;
    /// Distribution share for the Insurance fund, in basis points (1,000 = 10%).
    pub(crate) const INSURANCE_BPS: u32 = 1000;
    /// Distribution share for the Dividend fund, in basis points (400 = 4%).
    pub(crate) const DIVIDEND_BPS: u32 = 400;
    /// Distribution share for the Buyback fund, in basis points (400 = 4%).
    pub(crate) const BUYBACK_BPS: u32 = 400;
    /// Distribution share for the Voting fund, in basis points (200 = 2%).
    pub(crate) const VOTING_BPS: u32 = 200;
    /// Sum of all distribution BPS; must equal 10,000 (100%) at construction time.
    pub(crate) const TOTAL_BPS: u32 = 10_000;

    /// Six flat treasury buckets. Incoming fees are split across these by the documented basis points.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub enum Fund {
        Emergency,
        Operation,
        Insurance,
        Dividend,
        Buyback,
        Voting,
    }

    /// Which asset to release: the TUSDT stablecoin or the native token.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    #[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
    pub enum TokenKind {
        Tusdt,
        Native,
    }

    /// Treasury storage: governance role, the TUSDT token ref, and per-fund booked balances.
    #[ink(storage)]
    pub struct TusdtTreasury {
        governance: AccountId,
        token: TusdtErc20Ref,
        tusdt_allocated: Balance,
        native_allocated: Balance,
        funds_tusdt: Mapping<Fund, Balance>,
        funds_native: Mapping<Fund, Balance>,
    }

    /// Emitted when pending TUSDT and/or native balances are booked into the six funds.
    #[ink(event)]
    pub struct FundsDistributed {
        tusdt_delta: Balance,
        native_delta: Balance,
    }

    /// Emitted when the governance authorizes a withdrawal from a named fund.
    #[ink(event)]
    pub struct FundReleased {
        #[ink(topic)]
        fund: Fund,
        token_kind: TokenKind,
        amount: Balance,
        #[ink(topic)]
        recipient: AccountId,
    }

    /// Emitted when the current governance reassigns the governance address.
    #[ink(event)]
    pub struct GovernanceUpdated {
        previous: AccountId,
        new: AccountId,
    }

    /// Errors returned by the treasury contract.
    #[derive(Debug, PartialEq, Eq)]
    #[ink::scale_derive(Encode, Decode, TypeInfo)]
    pub enum Error {
        /// The caller is not the governance account.
        NotGovernance,
        /// The fund's balance is insufficient for the requested release amount.
        InsufficientFundBalance,
        /// The ERC20 or native transfer call failed.
        TransferFailed,
        /// An arithmetic overflow or underflow occurred.
        ArithmeticError,
    }

    pub type Result<T> = core::result::Result<T, Error>;

    impl TusdtTreasury {
        /// Initializes the treasury with the deployer as the initial governance and a TUSDT token address.
        ///
        /// The token ref is constructed from `token` via `FromAccountId`. The deployer can later hand
        /// off control to the governance contract by calling `set_governance`.
        #[ink(constructor)]
        pub fn new(token_address: AccountId) -> Self {
            let token = TusdtErc20Ref::from_account_id(token_address);
            Self {
                governance: Self::env().caller(),
                token,
                tusdt_allocated: 0,
                native_allocated: 0,
                funds_tusdt: Mapping::default(),
                funds_native: Mapping::default(),
            }
        }

        /// Returns the governance account permitted to release funds and reassign the role.
        #[ink(message)]
        pub fn governance(&self) -> AccountId {
            self.governance
        }

        /// Returns the TUSDT token contract address.
        #[ink(message)]
        pub fn token(&self) -> AccountId {
            ink::ToAccountId::to_account_id(&self.token)
        }

        /// Returns the booked TUSDT balance for a fund.
        #[ink(message)]
        pub fn fund_balance_tusdt(&self, fund: Fund) -> Balance {
            self.funds_tusdt.get(fund).unwrap_or_default()
        }

        /// Returns the booked native balance for a fund.
        #[ink(message)]
        pub fn fund_balance_native(&self, fund: Fund) -> Balance {
            self.funds_native.get(fund).unwrap_or_default()
        }

        /// Returns the unbooked TUSDT delta currently sitting at the treasury account.
        #[ink(message)]
        pub fn pending_tusdt(&self) -> Balance {
            let actual = self.token.balance_of(self.env().account_id());
            actual.saturating_sub(self.tusdt_allocated)
        }

        /// Returns the unbooked native delta currently sitting at the treasury account.
        #[ink(message)]
        pub fn pending_native(&self) -> Balance {
            self.env().balance().saturating_sub(self.native_allocated)
        }

        /// Reassigns the governance role; callable only by the current governance. Used at wiring time
        /// to hand off from the deployer to the governance contract.
        #[ink(message)]
        pub fn set_governance(&mut self, new_governance: AccountId) -> Result<()> {
            self.ensure_governance()?;
            let previous = self.governance;
            self.governance = new_governance;
            self.env().emit_event(GovernanceUpdated {
                previous,
                new: new_governance,
            });
            Ok(())
        }

        /// Books any new TUSDT and native deltas into the six funds by the documented basis points.
        ///
        /// Permissionless and idempotent. The integer-division remainder lands in `Emergency` so the
        /// books always balance against actual on-chain balances.
        #[ink(message)]
        pub fn distribute(&mut self) -> Result<()> {
            let actual_tusdt = self.token.balance_of(self.env().account_id());
            let actual_native = self.env().balance();

            let tusdt_delta = actual_tusdt.saturating_sub(self.tusdt_allocated);
            let native_delta = actual_native.saturating_sub(self.native_allocated);

            if tusdt_delta > 0 {
                self.allocate_to_funds(tusdt_delta, true)?;
                self.tusdt_allocated = actual_tusdt;
            }
            if native_delta > 0 {
                self.allocate_to_funds(native_delta, false)?;
                self.native_allocated = actual_native;
            }

            if tusdt_delta > 0 || native_delta > 0 {
                self.env().emit_event(FundsDistributed {
                    tusdt_delta,
                    native_delta,
                });
            }

            Ok(())
        }

        /// Releases `amount` of the named asset from `fund` to `recipient`; only callable by governance.
        ///
        /// Calls `distribute()` first so any newly-arrived fees are booked before the withdrawal.
        #[ink(message)]
        pub fn release(
            &mut self,
            fund: Fund,
            token_kind: TokenKind,
            amount: Balance,
            recipient: AccountId,
        ) -> Result<()> {
            self.ensure_governance()?;
            self.distribute()?;

            if amount == 0 {
                return Ok(());
            }

            match token_kind {
                TokenKind::Tusdt => {
                    let current = self.funds_tusdt.get(fund).unwrap_or_default();
                    if current < amount {
                        return Err(Error::InsufficientFundBalance);
                    }
                    // current >= amount checked above
                    #[allow(clippy::arithmetic_side_effects)]
                    self.funds_tusdt.insert(fund, &(current - amount));
                    self.tusdt_allocated = self
                        .tusdt_allocated
                        .checked_sub(amount)
                        .ok_or(Error::ArithmeticError)?;
                    self.token
                        .transfer(recipient, amount)
                        .map_err(|_| Error::TransferFailed)?;
                }
                TokenKind::Native => {
                    let current = self.funds_native.get(fund).unwrap_or_default();
                    if current < amount {
                        return Err(Error::InsufficientFundBalance);
                    }
                    #[allow(clippy::arithmetic_side_effects)]
                    self.funds_native.insert(fund, &(current - amount));
                    self.native_allocated = self
                        .native_allocated
                        .checked_sub(amount)
                        .ok_or(Error::ArithmeticError)?;
                    self.env()
                        .transfer(recipient, amount)
                        .map_err(|_| Error::TransferFailed)?;
                }
            }

            self.env().emit_event(FundReleased {
                fund,
                token_kind,
                amount,
                recipient,
            });
            Ok(())
        }

        /// Splits `delta` across the six funds by documented bps; remainder is added to `Emergency`.
        fn allocate_to_funds(&mut self, delta: Balance, is_tusdt: bool) -> Result<()> {
            let shares = split_delta(delta).ok_or(Error::ArithmeticError)?;
            let funds = [
                (Fund::Emergency, shares[0]),
                (Fund::Operation, shares[1]),
                (Fund::Insurance, shares[2]),
                (Fund::Dividend, shares[3]),
                (Fund::Buyback, shares[4]),
                (Fund::Voting, shares[5]),
            ];
            for (fund, share) in funds {
                if share == 0 {
                    continue;
                }
                if is_tusdt {
                    let current = self.funds_tusdt.get(fund).unwrap_or_default();
                    let next = current.checked_add(share).ok_or(Error::ArithmeticError)?;
                    self.funds_tusdt.insert(fund, &next);
                } else {
                    let current = self.funds_native.get(fund).unwrap_or_default();
                    let next = current.checked_add(share).ok_or(Error::ArithmeticError)?;
                    self.funds_native.insert(fund, &next);
                }
            }
            Ok(())
        }

        /// Ensures the caller is the current governance account. Reverts with `NotGovernance` otherwise.
        fn ensure_governance(&self) -> Result<()> {
            if self.env().caller() != self.governance {
                return Err(Error::NotGovernance);
            }
            Ok(())
        }
    }

    /// Pure split of `delta` into `[Emergency, Operation, Insurance, Dividend, Buyback, Voting]` shares.
    ///
    /// Each share is `delta * bps / 10_000`; any integer-division remainder is credited to Emergency
    /// so the six shares sum exactly to `delta`. Each share is bounded above by `delta` (a `Balance`),
    /// so the `u128 -> Balance` downcasts via `try_from` cannot fail; we still go through `try_from`
    /// to keep clippy quiet about cast truncation.
    pub(crate) fn split_delta(delta: Balance) -> Option<[Balance; 6]> {
        let total_bps = u128::from(TOTAL_BPS);
        let d = u128::from(delta);
        let op = d.checked_mul(u128::from(OPERATION_BPS))?.checked_div(total_bps)?;
        let ins = d.checked_mul(u128::from(INSURANCE_BPS))?.checked_div(total_bps)?;
        let div = d.checked_mul(u128::from(DIVIDEND_BPS))?.checked_div(total_bps)?;
        let buy = d.checked_mul(u128::from(BUYBACK_BPS))?.checked_div(total_bps)?;
        let vot = d.checked_mul(u128::from(VOTING_BPS))?.checked_div(total_bps)?;
        let assigned = op
            .checked_add(ins)?
            .checked_add(div)?
            .checked_add(buy)?
            .checked_add(vot)?;
        let emergency = d.checked_sub(assigned)?;
        Some([
            Balance::try_from(emergency).ok()?,
            Balance::try_from(op).ok()?,
            Balance::try_from(ins).ok()?,
            Balance::try_from(div).ok()?,
            Balance::try_from(buy).ok()?,
            Balance::try_from(vot).ok()?,
        ])
    }
}

#[cfg(test)]
mod tests;
