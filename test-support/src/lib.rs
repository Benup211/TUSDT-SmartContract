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
//!   2|5|6 = no-op write success, 25 = caller_transfer_stake, anything else = read
//!   failure), netuid 1, hotkey `bob`, coldkey `alice`.
//! - [`MockExtension::subnet_stake`] — oracle / election style: every function id
//!   answers with a `StakeInfo` record, netuid 113, hotkey/coldkey `alice`.
//!
//! Contracts whose copies needed bespoke behaviour (e.g. oracle's `is_registered`
//! knob) build their own variant from the knobs and install it with
//! [`register_extension`].

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
/// public knobs and the builder methods. Install the finished mock with
/// [`register_extension`] (or one of the `register_mock_*` conveniences).
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
    /// copies); when `false`, the vault/lending dispatch table applies (0/15/36/2|5|6/25,
    /// anything else failing with status 1).
    pub answers_any_func_id: bool,
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

impl test::ChainExtension for MockExtension {
    fn ext_id(&self) -> u16 {
        0x1000
    }

    fn call(&mut self, func_id: u16, _input: &[u8], output: &mut Vec<u8>) -> u32 {
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
            // get_stake_availability
            36 => {
                let availability = tusdt_env::StakeAvailability {
                    netuid: self.netuid,
                    total: self.stake.unwrap_or(0),
                    locked: 0,
                    available: self.stake.unwrap_or(0),
                };
                ink::scale::Encode::encode_to(&availability, output);
                0
            },
            // Write ops (2 = remove_stake, 5 = move_stake, 6 = transfer_stake) — no-op success
            2 | 5 | 6 => 0,
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
