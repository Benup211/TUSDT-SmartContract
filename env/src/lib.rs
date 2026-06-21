#![cfg_attr(not(feature = "std"), no_std)]
//! Custom ink! environment for the tUSDT protocol. Defines [`CustomEnvironment`] with
//! `Balance = u64`, `Timestamp = u64` (Unix ms), and `BlockNumber = u32`, plus a 16-function
//! chain extension (id `0x1000`) exposing Bittensor subnet staking operations and alpha price
//! queries. All protocol contracts use this environment via
//! `#[ink::contract(env = tusdt_env::CustomEnvironment)]`.

use parity_scale_codec::Compact;

/// Custom ink! environment used by all tUSDT protocol contracts. Differs from `DefaultEnvironment`
/// in using `Balance = u64`, `Timestamp = u64` (Unix epoch milliseconds), and `BlockNumber = u32`,
/// with a 16-function chain extension for Bittensor subnet operations.
#[derive(Debug, Clone)]
pub struct CustomEnvironment;

/// Identifiers for each function in the chain extension at extension id `0x1000`, mapping 1:1
/// to the function indices in `RuntimeReadWrite`.
pub enum FunctionId {
    /// Reads stake info for a (hotkey, coldkey, netuid) triplet (function 0).
    GetStakeInfoForHotkeyColdkeyNetuidV1 = 0,
    /// Adds stake to a hotkey on a subnet (function 1).
    AddStakeV1 = 1,
    /// Removes stake from a hotkey on a subnet (function 2).
    RemoveStakeV1 = 2,
    /// Unstakes all TAO from a hotkey (function 3).
    UnstakeAllV1 = 3,
    /// Unstakes all alpha from a hotkey (function 4).
    UnstakeAllAlphaV1 = 4,
    /// Moves stake between hotkeys across subnets (function 5).
    MoveStakeV1 = 5,
    /// Transfers stake to a different coldkey (function 6).
    TransferStakeV1 = 6,
    /// Swaps stake across subnets for the same hotkey (function 7).
    SwapStakeV1 = 7,
    /// Adds stake with a price limit (function 8).
    AddStakeLimitV1 = 8,
    /// Removes stake with a price limit (function 9).
    RemoveStakeLimitV1 = 9,
    /// Swaps stake with a price limit (function 10).
    SwapStakeLimitV1 = 10,
    /// Removes all stake with a price limit (function 11).
    RemoveStakeFullLimitV1 = 11,
    /// Sets a coldkey's auto-stake hotkey for a subnet (function 12).
    SetColdkeyAutoStakeHotkeyV1 = 12,
    /// Adds a proxy delegate (function 13).
    AddProxyV1 = 13,
    /// Removes a proxy delegate (function 14).
    RemoveProxyV1 = 14,
    /// Reads the alpha price for a subnet (function 15).
    GetAlphaPriceV1 = 15,
}

/// The 16-function chain extension at extension id `0x1000`. Exposes Bittensor subnet staking
/// operations (add/remove/move/swap stake, stake limits, proxies, auto-stake) and alpha price
/// queries. Functions 0 and 15 are read-only; functions 1–14 are write operations.
#[ink::chain_extension(extension = 0x1000)]
pub trait RuntimeReadWrite {
    type ErrorCode = ReadWriteErrorCode;

    /// Queries stake information for a (hotkey, coldkey) pair on a given subnet. Returns the full
    /// `StakeInfo` record including stake, locked, emission, drain, and registration status.
    #[ink(function = 0)]
    fn get_stake_info_for_hotkey_coldkey_netuid(
        hotkey: ink::primitives::AccountId,
        coldkey: ink::primitives::AccountId,
        netuid: u16,
    ) -> Option<StakeInfo<ink::primitives::AccountId>>;

    /// Adds stake to a hotkey on the specified subnet.
    #[ink(function = 1)]
    fn add_stake(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        netuid: u16,
        amount: u64,
    );

    /// Removes stake from a hotkey on the specified subnet.
    #[ink(function = 2)]
    fn remove_stake(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        netuid: u16,
        amount: u64,
    );

    /// Unstakes all TAO from a hotkey.
    #[ink(function = 3)]
    fn unstake_all(hotkey: <CustomEnvironment as ink::env::Environment>::AccountId);

    /// Unstakes all alpha from a hotkey.
    #[ink(function = 4)]
    fn unstake_all_alpha(hotkey: <CustomEnvironment as ink::env::Environment>::AccountId);

    /// Moves stake between hotkeys across subnets.
    #[ink(function = 5)]
    fn move_stake(
        origin_hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        destination_hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        origin_netuid: u16,
        destination_netuid: u16,
        amount: u64,
    );

    /// Transfers stake to a different coldkey.
    #[ink(function = 6)]
    fn transfer_stake(
        destination_coldkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        origin_netuid: u16,
        destination_netuid: u16,
        amount: u64,
    );

    /// Swaps stake across subnets for the same hotkey.
    #[ink(function = 7)]
    fn swap_stake(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        origin_netuid: u16,
        destination_netuid: u16,
        amount: u64,
    );

    /// Adds stake with a price limit.
    #[ink(function = 8)]
    fn add_stake_limit(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        netuid: u16,
        amount: u64,
        limit_price: u64,
        allow_partial: bool,
    );

    /// Removes stake with a price limit.
    #[ink(function = 9)]
    fn remove_stake_limit(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        netuid: u16,
        amount: u64,
        limit_price: u64,
        allow_partial: bool,
    );

    /// Swaps stake with a price limit.
    #[ink(function = 10)]
    fn swap_stake_limit(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        origin_netuid: u16,
        destination_netuid: u16,
        amount: u64,
        limit_price: u64,
        allow_partial: bool,
    );

    /// Removes all stake with a price limit.
    #[ink(function = 11)]
    fn remove_stake_full_limit(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        netuid: u16,
        limit_price: u64,
    );

    /// Sets a coldkey's auto-stake hotkey for a subnet.
    #[ink(function = 12)]
    fn set_coldkey_auto_stake_hotkey(
        netuid: u16,
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
    );

    /// Adds a proxy delegate.
    #[ink(function = 13)]
    fn add_proxy(delegate: <CustomEnvironment as ink::env::Environment>::AccountId);

    /// Removes a proxy delegate.
    #[ink(function = 14)]
    fn remove_proxy(delegate: <CustomEnvironment as ink::env::Environment>::AccountId);

    /// Reads the current alpha price for the specified subnet.
    #[ink(function = 15)]
    fn get_alpha_price(netuid: u16) -> u64;
}

/// Error codes returned by the chain extension. Status code 0 indicates success; 1 indicates
/// a read failure; 2 indicates a write failure.
#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub enum ReadWriteErrorCode {
    /// The chain extension read operation failed (status code 1).
    ReadFailed,
    /// The chain extension write operation failed (status code 2).
    WriteFailed,
}

impl ink::env::chain_extension::FromStatusCode for ReadWriteErrorCode {
    fn from_status_code(status_code: u32) -> Result<(), Self> {
        match status_code {
            0 => Ok(()),
            1 => Err(ReadWriteErrorCode::ReadFailed),
            2 => Err(ReadWriteErrorCode::WriteFailed),
            _ => Err(ReadWriteErrorCode::ReadFailed),
        }
    }
}

impl ink::env::Environment for CustomEnvironment {
    const MAX_EVENT_TOPICS: usize = 4;

    type AccountId = ink::primitives::AccountId;
    type Balance = u64;
    type Hash = ink::primitives::Hash;
    type BlockNumber = u32;
    type Timestamp = u64;

    type ChainExtension = RuntimeReadWrite;
}

/// Stake information for a (hotkey, coldkey) subnet pair, returned by the chain extension's
/// `get_stake_info` function. Fields use SCALE `Compact` encoding for the u64 quantities.
#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub struct StakeInfo<AccountId> {
    /// The hotkey account.
    pub hotkey: AccountId,
    /// The coldkey account that owns the stake.
    pub coldkey: AccountId,
    /// The subnet identifier (SCALE compact).
    pub netuid: Compact<u16>,
    /// The total alpha stake held on this subnet (SCALE compact).
    pub stake: Compact<u64>,
    /// The locked (non-transferable) portion of the stake (SCALE compact).
    pub locked: Compact<u64>,
    /// Accumulated alpha emissions for this stake (SCALE compact).
    pub emission: Compact<u64>,
    /// Accumulated TAO emissions for this stake (SCALE compact).
    pub tao_emission: Compact<u64>,
    /// The stake drain rate (SCALE compact).
    pub drain: Compact<u64>,
    /// Whether the (hotkey, coldkey) pair is registered on the subnet.
    pub is_registered: bool,
}
