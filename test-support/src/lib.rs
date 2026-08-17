//! Shared off-chain test infrastructure for the tUSDT protocol contracts.
//!
//! This crate hosts the single unified chain-extension mock (`MockExtension`) plus the
//! `register_*` helpers that install it into the ink! off-chain test environment. It is
//! test infrastructure only: the contracts wire it in as a dev-dependency, and it is
//! never part of an on-chain build.
//!
//! The mock reproduces the four per-contract copies that previously lived in the
//! `tests.rs` files, unified behind two construction styles plus public field knobs:
//!
//! - [`MockExtension::dispatch`] — vault / lending-pool style: full function-id
//!   dispatch table (0 = stake info, 15 = alpha price, 36 = stake availability,
//!   1 = add_stake, 2 = remove_stake, 5|6 = no-op write success,
//!   25 = caller_transfer_stake, anything else = read failure), netuid 1,
//!   hotkey `bob`, coldkey `alice`.
//! - [`MockExtension::subnet_stake`] — oracle / election style: every function id
//!   answers with a `StakeInfo` record, netuid 113, hotkey/coldkey `alice`.
//!
//! Contracts whose copies needed bespoke behaviour (e.g. oracle's `is_registered`
//! knob) build their own variant from the knobs and install it with
//! [`register_extension`].
//!
//! The mock also simulates idle TAO root-subnet staking. Func 1 (`add_stake`) and
//! func 2 (`remove_stake`) are no-op successes by default; with
//! [`MockExtension::with_stateful_root_stake`] (or [`register_mock_stateful_root`])
//! they decode `(hotkey, netuid, amount)` and move the off-chain callee's balance in
//! and out of the simulated root stake, and func 36 (`get_stake_availability`)
//! reports that simulated `root_stake` for netuid 0.

// Test infrastructure is intentionally ergonomic: the workspace-wide panic-free lints
// (unwrap/expect/indexing/arithmetic) would otherwise fight mock plumbing that is only
// ever exercised by tests.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

pub use tusdt_env::StakeInfo;

use ink::env::test;
use ink::primitives::AccountId;
use ink::scale::Compact;

/// Unified mock for the tUSDT chain extension (extension id `0x1000`).
///
/// Construct it with [`MockExtension::dispatch`] (vault/lending-pool behaviour) or
/// [`MockExtension::subnet_stake`] (oracle/election behaviour), then fine-tune via the
/// public knobs and the builder methods. The stateful root-stake mode
/// ([`MockExtension::with_stateful_root_stake`], or the [`register_mock_stateful_root`]
/// convenience) makes funcs 1/2 move the off-chain callee's balance in and out of a
/// simulated root stake and makes func 36 report that stake for netuid 0. Install the
/// finished mock with [`register_extension`] (or one of the `register_mock_*`
/// conveniences).
pub struct MockExtension {
    /// Alpha stake reported by `get_stake_info` (func 0). `None` encodes a missing
    /// stake record for the queried triplet.
    pub stake: Option<u64>,
    /// `is_registered` field of the reported `StakeInfo` (oracle tests drive this knob).
    pub is_registered: bool,
    /// When `true`, every call fails with status 1 (`ReadFailed`).
    pub should_fail: bool,
    /// When `true`, func 25 (`caller_transfer_stake`) fails with status 2 (`WriteFailed`).
    pub transfer_fails: bool,
    /// `netuid` reported inside `StakeInfo` (vault/lending: 1; oracle/election: 113).
    pub netuid: u16,
    /// Hotkey reported inside `StakeInfo` (vault/lending: `bob`; oracle/election: `alice`).
    pub hotkey: AccountId,
    /// Coldkey reported inside `StakeInfo` (all copies: `alice`).
    pub coldkey: AccountId,
    /// When `true`, every func id answers with the `StakeInfo` record (oracle/election
    /// copies); when `false`, the vault/lending dispatch table applies (0/15/36/1/2/5|6/25,
    /// anything else failing with status 1).
    pub answers_any_func_id: bool,
    /// Simulated idle TAO staked on the root subnet (netuid 0): reported by func 36 and
    /// moved by funcs 1/2 when `move_balances` is enabled.
    pub root_stake: u64,
    /// When `true`, func 1 (`add_stake`) and func 2 (`remove_stake`) decode their
    /// arguments and move the off-chain callee's balance in and out of `root_stake`;
    /// func 36 then reports `root_stake` for netuid 0.
    pub move_balances: bool,
    /// When `true`, func 1 (`add_stake`) fails with status 2 (`WriteFailed`).
    pub add_stake_fails: bool,
    /// When `true`, func 2 (`remove_stake`) fails with status 2 (`WriteFailed`).
    pub remove_stake_fails: bool,
}

impl MockExtension {
    /// Vault/lending-pool style mock: full dispatch table, netuid 1, hotkey `bob`,
    /// coldkey `alice`, registered.
    pub fn dispatch(stake: Option<u64>) -> Self {
        let accounts = default_accounts();
        Self {
            stake,
            is_registered: true,
            should_fail: false,
            transfer_fails: false,
            netuid: 1,
            hotkey: accounts.bob,
            coldkey: accounts.alice,
            answers_any_func_id: false,
            root_stake: 0,
            move_balances: false,
            add_stake_fails: false,
            remove_stake_fails: false,
        }
    }

    /// Oracle/election style mock: every func id answers with the `StakeInfo` record,
    /// netuid 113, hotkey/coldkey `alice`, registered.
    pub fn subnet_stake(stake: Option<u64>) -> Self {
        let accounts = default_accounts();
        Self {
            stake,
            is_registered: true,
            should_fail: false,
            transfer_fails: false,
            netuid: 113,
            hotkey: accounts.alice,
            coldkey: accounts.alice,
            answers_any_func_id: true,
            root_stake: 0,
            move_balances: false,
            add_stake_fails: false,
            remove_stake_fails: false,
        }
    }

    /// Builder knob: `is_registered` reported inside `StakeInfo`.
    pub fn with_is_registered(mut self, value: bool) -> Self {
        self.is_registered = value;
        self
    }

    /// Builder knob: fail every call with status 1 (`ReadFailed`).
    pub fn with_should_fail(mut self, value: bool) -> Self {
        self.should_fail = value;
        self
    }

    /// Builder knob: fail func 25 (`caller_transfer_stake`) with status 2 (`WriteFailed`).
    pub fn with_transfer_fails(mut self, value: bool) -> Self {
        self.transfer_fails = value;
        self
    }

    /// Builder knob: simulate stateful idle TAO root-subnet staking. Enables
    /// `move_balances` and initialises `root_stake` to `initial`; funcs 1/2 then move
    /// the off-chain callee's balance in and out of the simulated root stake and func 36
    /// reports it for netuid 0.
    pub fn with_stateful_root_stake(mut self, initial: u64) -> Self {
        self.move_balances = true;
        self.root_stake = initial;
        self
    }

    /// Builder knob: fail func 1 (`add_stake`) with status 2 (`WriteFailed`).
    pub fn with_add_stake_fails(mut self, value: bool) -> Self {
        self.add_stake_fails = value;
        self
    }

    /// Builder knob: fail func 2 (`remove_stake`) with status 2 (`WriteFailed`).
    pub fn with_remove_stake_fails(mut self, value: bool) -> Self {
        self.remove_stake_fails = value;
        self
    }

    fn stake_info(&self) -> Option<StakeInfo<AccountId>> {
        self.stake.map(|stake| StakeInfo {
            hotkey: self.hotkey,
            coldkey: self.coldkey,
            netuid: Compact(self.netuid),
            stake: Compact(stake),
            locked: Compact(0),
            emission: Compact(0),
            tao_emission: Compact(0),
            drain: Compact(0),
            is_registered: self.is_registered,
        })
    }
}

/// Decodes a chain-extension argument tuple from the raw input buffer.
///
/// The off-chain engine wraps the argument bytes in a length-prefixed `Vec<u8>`
/// (`ink_engine::ext::Engine::call_chain_extension` calls `input.encode()`), so
/// the prefix must be stripped before decoding the tuple.
fn decode_input<T: ink::scale::Decode>(input: &[u8]) -> Result<T, ()> {
    let inner = <Vec<u8> as ink::scale::Decode>::decode(&mut &input[..]).map_err(|_| ())?;
    T::decode(&mut &inner[..]).map_err(|_| ())
}

impl test::ChainExtension for MockExtension {
    fn ext_id(&self) -> u16 {
        0x1000
    }

    fn call(&mut self, func_id: u16, input: &[u8], output: &mut Vec<u8>) -> u32 {
        if self.should_fail {
            return 1; // ReadFailed
        }
        if self.answers_any_func_id {
            // Oracle/election copies answered every function with the stake record.
            ink::scale::Encode::encode_to(&self.stake_info(), output);
            return 0;
        }
        match func_id {
            // get_stake_info_for_hotkey_coldkey_netuid
            0 => {
                ink::scale::Encode::encode_to(&self.stake_info(), output);
                0
            },
            // get_alpha_price — 1_000_000_000 = 1 alpha = 1 TAO
            15 => {
                let price: u64 = 1_000_000_000;
                ink::scale::Encode::encode_to(&price, output);
                0
            },
            // add_stake — idle TAO root-subnet staking
            1 => {
                let Ok((hotkey, netuid, amount)) = decode_input::<(AccountId, u16, u64)>(input)
                else {
                    return 1; // ReadFailed: malformed input
                };
                if self.add_stake_fails {
                    return 2; // WriteFailed
                }
                if self.move_balances {
                    self.root_stake = self.root_stake.saturating_add(amount);
                }
                record_call(ExtCallRecord { func_id: 1, hotkey, netuid, amount });
                0
            },
            // remove_stake — idle TAO root-subnet unstaking
            2 => {
                if self.remove_stake_fails {
                    return 2; // WriteFailed
                }
                let Ok((hotkey, netuid, amount)) = decode_input::<(AccountId, u16, u64)>(input)
                else {
                    return 1; // ReadFailed: malformed input
                };
                if self.move_balances {
                    let take = amount.min(self.root_stake);
                    self.root_stake = self.root_stake.saturating_sub(take);
                    record_call(ExtCallRecord { func_id: 2, hotkey, netuid, amount: take });
                } else {
                    record_call(ExtCallRecord { func_id: 2, hotkey, netuid, amount });
                }
                0
            },
            // get_stake_availability
            36 => {
                if self.move_balances {
                    let Ok((_coldkey, netuid)) = decode_input::<(AccountId, u16)>(input) else {
                        return 1; // ReadFailed: malformed input
                    };
                    record_call(ExtCallRecord {
                        func_id: 36,
                        hotkey: AccountId::from([0u8; 32]),
                        netuid,
                        amount: self.root_stake,
                    });
                    record_raw_input(input);
                    if netuid == 0 {
                        // Simulated idle TAO stake on the root subnet.
                        let availability = tusdt_env::StakeAvailability {
                            netuid: 0,
                            total: self.root_stake,
                            locked: 0,
                            available: self.root_stake,
                        };
                        ink::scale::Encode::encode_to(&availability, output);
                        return 0;
                    }
                }
                let availability = tusdt_env::StakeAvailability {
                    netuid: self.netuid,
                    total: self.stake.unwrap_or(0),
                    locked: 0,
                    available: self.stake.unwrap_or(0),
                };
                ink::scale::Encode::encode_to(&availability, output);
                0
            },
            // Write ops (5 = move_stake, 6 = transfer_stake) — no-op success
            5 | 6 => 0,
            // 25 = caller_transfer_stake — honours transfer_fails
            25 => {
                if self.transfer_fails {
                    2 // WriteFailed
                } else {
                    0
                }
            },
            _ => 1,
        }
    }
}

fn default_accounts() -> test::DefaultAccounts<tusdt_env::CustomEnvironment> {
    test::default_accounts::<tusdt_env::CustomEnvironment>()
}

/// Reads the off-chain balance of `account` as `u64`.
///
/// `ink::env::test::{get,set}_account_balance` only support environments with
/// `Balance = u128`, while `tusdt_env::CustomEnvironment` uses `Balance = u64`. The
/// off-chain engine stores balances keyed by account id alone, so calling through
/// `ink::env::DefaultEnvironment` reads and writes exactly the same storage that
/// `ink::env::balance::<tusdt_env::CustomEnvironment>()` (i.e. `env().balance()`)
/// observes. Balances above `u64::MAX` — uncreatable through `CustomEnvironment`
/// contract paths — saturate.
pub fn callee_balance(account: AccountId) -> u64 {
    u64::try_from(
        ink::env::test::get_account_balance::<ink::env::DefaultEnvironment>(account).unwrap_or(0),
    )
    .unwrap_or(u64::MAX)
}

/// Sets the off-chain balance of `account` from a `u64` value. See [`callee_balance`]
/// for why the `DefaultEnvironment` host type is used.
pub fn set_callee_balance(account: AccountId, balance: u64) {
    ink::env::test::set_account_balance::<ink::env::DefaultEnvironment>(
        account,
        u128::from(balance),
    );
}

/// Sets the off-chain test caller (and callee) to `caller`.
pub fn set_caller(caller: AccountId) {
    let callee = ink::env::account_id::<tusdt_env::CustomEnvironment>();
    ink::env::test::set_callee::<tusdt_env::CustomEnvironment>(callee);
    ink::env::test::set_caller::<tusdt_env::CustomEnvironment>(caller);
}

/// Registers a fully custom [`MockExtension`] with the off-chain test environment.
pub fn register_extension(ext: MockExtension) {
    test::register_chain_extension(ext);
}

/// Registers the vault/lending-pool dispatch-style mock with `Some(stake)`.
pub fn register_mock(stake: u64) {
    register_extension(MockExtension::dispatch(Some(stake)));
}

/// Registers the dispatch-style mock with no stake record (`None`).
pub fn register_mock_no_stake() {
    register_extension(MockExtension::dispatch(None));
}

/// Registers the dispatch-style mock where func 25 (`caller_transfer_stake`) fails with
/// a write error; `stake` is the alpha stake reported for the queried triplet.
pub fn register_mock_transfer_fails(stake: u64) {
    register_extension(MockExtension::dispatch(Some(stake)).with_transfer_fails(true));
}

/// Registers the dispatch-style mock where every call fails with a read error
/// (reported stake 1_000, matching the original lending-pool helper).
pub fn register_mock_chain_fails() {
    register_extension(MockExtension::dispatch(Some(1_000)).with_should_fail(true));
}

/// Registers the dispatch-style mock in stateful root-stake mode: `root_stake`
/// starts at `initial`, funcs 1/2 (`add_stake`/`remove_stake`) track the simulated
/// root stake, and func 36 (`get_stake_availability`) reports it for netuid 0. No
/// alpha stake record (`None`).
///
/// The mock deliberately does NOT touch off-chain account balances: the chain
/// extension executes while the off-chain engine holds its internal borrow, so any
/// `ink::env::test` balance API called from inside the mock would panic with
/// "RefCell already borrowed". Tests instead assert against [`last_ext_call`] and
/// the contract's own `staked_tao` bookkeeping (root is 1:1, so requested ==
/// received).
pub fn register_mock_stateful_root(initial: u64) {
    register_extension(MockExtension::dispatch(None).with_stateful_root_stake(initial));
}

/// A snapshot of the most recent successful root-staking chain-extension call
/// (funcs 1, 2, and 36), recorded for test assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtCallRecord {
    /// Chain-extension function id: 1 = add_stake, 2 = remove_stake, 36 = get_stake_availability.
    pub func_id: u16,
    /// Hotkey argument (`[0u8; 32]` placeholder for func 36, which has no hotkey).
    pub hotkey: AccountId,
    /// Netuid argument.
    pub netuid: u16,
    /// Amount argument (stake moved for 1/2, reported `available` for 36).
    pub amount: u64,
}

thread_local! {
    static LAST_EXT_CALL: std::cell::RefCell<Option<ExtCallRecord>> =
        const { std::cell::RefCell::new(None) };
    static LAST_RAW_INPUT: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn record_call(record: ExtCallRecord) {
    LAST_EXT_CALL.with(|cell| *cell.borrow_mut() = Some(record));
}

fn record_raw_input(input: &[u8]) {
    LAST_RAW_INPUT.with(|cell| *cell.borrow_mut() = input.to_vec());
}

/// Returns the raw SCALE input bytes of the most recent recorded extension call.
pub fn last_raw_input() -> Vec<u8> {
    LAST_RAW_INPUT.with(|cell| cell.borrow().clone())
}

/// Returns the most recent successful root-staking extension call, if any.
pub fn last_ext_call() -> Option<ExtCallRecord> {
    LAST_EXT_CALL.with(|cell| *cell.borrow())
}

/// Clears the recorded extension call (call at the start of each test).
pub fn reset_ext_calls() {
    LAST_EXT_CALL.with(|cell| *cell.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink::env::test::ChainExtension;
    use ink::scale::{Decode, Encode};

    fn encode_input<T: Encode>(value: &T) -> Vec<u8> {
        let mut buf = Vec::new();
        Encode::encode_to(value, &mut buf);
        buf
    }

    /// Encodes a chain-extension argument tuple the way the off-chain engine
    /// does: the encoded tuple wrapped in a length-prefixed `Vec<u8>`.
    fn encode_ext_input<T: Encode>(value: &T) -> Vec<u8> {
        encode_input(&value.encode())
    }

    #[test]
    fn stateful_add_stake_increments_root_stake_and_records_call() {
        let accounts = default_accounts();
        let mut ext = MockExtension::dispatch(None).with_stateful_root_stake(1_000);

        let input = encode_ext_input(&(accounts.bob, 0u16, 4_000u64));
        let mut output = Vec::new();
        let status = ext.call(1, &input, &mut output);

        assert_eq!(status, 0);
        assert_eq!(ext.root_stake, 5_000);
        let record = last_ext_call().unwrap();
        assert_eq!(record.func_id, 1);
        assert_eq!(record.hotkey, accounts.bob);
        assert_eq!(record.netuid, 0);
        assert_eq!(record.amount, 4_000);
    }

    #[test]
    fn stateful_remove_stake_caps_at_root_stake_and_records_call() {
        let accounts = default_accounts();
        let mut ext = MockExtension::dispatch(None).with_stateful_root_stake(1_000);

        // Removing more than the simulated stake only returns `root_stake`.
        let input = encode_ext_input(&(accounts.bob, 0u16, 4_000u64));
        let mut output = Vec::new();
        assert_eq!(ext.call(2, &input, &mut output), 0);
        assert_eq!(ext.root_stake, 0);
        let record = last_ext_call().unwrap();
        assert_eq!(record.func_id, 2);
        assert_eq!(record.amount, 1_000);

        // Nothing left to unstake: stake is unchanged and the call records zero.
        let input = encode_ext_input(&(accounts.bob, 0u16, 9_999u64));
        assert_eq!(ext.call(2, &input, &mut output), 0);
        assert_eq!(ext.root_stake, 0);
        assert_eq!(last_ext_call().unwrap().amount, 0);
    }

    #[test]
    fn stateful_func36_reports_root_stake_for_netuid_0() {
        let accounts = default_accounts();
        let mut ext = MockExtension::dispatch(None).with_stateful_root_stake(7_777);

        let input = encode_ext_input(&(accounts.alice, 0u16));
        let mut output = Vec::new();
        assert_eq!(ext.call(36, &input, &mut output), 0);
        let availability =
            <tusdt_env::StakeAvailability as Decode>::decode(&mut &output[..]).unwrap();
        assert_eq!(availability.netuid, 0);
        assert_eq!(availability.total, 7_777);
        assert_eq!(availability.locked, 0);
        assert_eq!(availability.available, 7_777);

        // Non-root netuids keep the legacy behaviour (alpha stake / mock netuid).
        let input = encode_ext_input(&(accounts.alice, 1u16));
        let mut output = Vec::new();
        assert_eq!(ext.call(36, &input, &mut output), 0);
        let availability =
            <tusdt_env::StakeAvailability as Decode>::decode(&mut &output[..]).unwrap();
        assert_eq!(availability.netuid, 1);
        assert_eq!(availability.total, 0); // dispatch(None): no alpha stake record
    }

    #[test]
    fn plain_mock_func1_is_noop_success() {
        let accounts = default_accounts();
        let mut ext = MockExtension::dispatch(Some(100));

        let input = encode_ext_input(&(accounts.bob, 0u16, 1_000u64));
        let mut output = Vec::new();
        assert_eq!(ext.call(1, &input, &mut output), 0);
        assert_eq!(ext.root_stake, 0);
    }
}
