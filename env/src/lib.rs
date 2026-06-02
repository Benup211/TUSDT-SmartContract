#![cfg_attr(not(feature = "std"), no_std)]

use parity_scale_codec::Compact;

#[derive(Debug, Clone)]
pub struct CustomEnvironment;

pub enum FunctionId {
    GetStakeInfoForHotkeyColdkeyNetuidV1 = 0,
    AddStakeV1 = 1,
    RemoveStakeV1 = 2,
    UnstakeAllV1 = 3,
    UnstakeAllAlphaV1 = 4,
    MoveStakeV1 = 5,
    TransferStakeV1 = 6,
    SwapStakeV1 = 7,
    AddStakeLimitV1 = 8,
    RemoveStakeLimitV1 = 9,
    SwapStakeLimitV1 = 10,
    RemoveStakeFullLimitV1 = 11,
    SetColdkeyAutoStakeHotkeyV1 = 12,
    AddProxyV1 = 13,
    RemoveProxyV1 = 14,
    GetAlphaPriceV1 = 15,
}

#[ink::chain_extension(extension = 0x1000)]
pub trait RuntimeReadWrite {
    type ErrorCode = ReadWriteErrorCode;

    #[ink(function = 0)]
    fn get_stake_info_for_hotkey_coldkey_netuid(
        hotkey: ink::primitives::AccountId,
        coldkey: ink::primitives::AccountId,
        netuid: u16,
    ) -> Option<StakeInfo<ink::primitives::AccountId>>;

    #[ink(function = 1)]
    fn add_stake(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        netuid: u16,
        amount: u64,
    );

    #[ink(function = 2)]
    fn remove_stake(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        netuid: u16,
        amount: u64,
    );

    #[ink(function = 3)]
    fn unstake_all(hotkey: <CustomEnvironment as ink::env::Environment>::AccountId);

    #[ink(function = 4)]
    fn unstake_all_alpha(hotkey: <CustomEnvironment as ink::env::Environment>::AccountId);

    #[ink(function = 5)]
    fn move_stake(
        origin_hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        destination_hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        origin_netuid: u16,
        destination_netuid: u16,
        amount: u64,
    );

    #[ink(function = 6)]
    fn transfer_stake(
        destination_coldkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        origin_netuid: u16,
        destination_netuid: u16,
        amount: u64,
    );

    #[ink(function = 7)]
    fn swap_stake(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        origin_netuid: u16,
        destination_netuid: u16,
        amount: u64,
    );

    #[ink(function = 8)]
    fn add_stake_limit(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        netuid: u16,
        amount: u64,
        limit_price: u64,
        allow_partial: bool,
    );

    #[ink(function = 9)]
    fn remove_stake_limit(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        netuid: u16,
        amount: u64,
        limit_price: u64,
        allow_partial: bool,
    );

    #[ink(function = 10)]
    fn swap_stake_limit(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        origin_netuid: u16,
        destination_netuid: u16,
        amount: u64,
        limit_price: u64,
        allow_partial: bool,
    );

    #[ink(function = 11)]
    fn remove_stake_full_limit(
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
        netuid: u16,
        limit_price: u64,
    );

    #[ink(function = 12)]
    fn set_coldkey_auto_stake_hotkey(
        netuid: u16,
        hotkey: <CustomEnvironment as ink::env::Environment>::AccountId,
    );

    #[ink(function = 13)]
    fn add_proxy(delegate: <CustomEnvironment as ink::env::Environment>::AccountId);

    #[ink(function = 14)]
    fn remove_proxy(delegate: <CustomEnvironment as ink::env::Environment>::AccountId);

    #[ink(function = 15)]
    fn get_alpha_price(netuid: u16) -> u64;
}

#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub enum ReadWriteErrorCode {
    ReadFailed,
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

#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub struct StakeInfo<AccountId> {
    pub hotkey: AccountId,
    pub coldkey: AccountId,
    pub netuid: Compact<u16>,
    pub stake: Compact<u64>,
    pub locked: Compact<u64>,
    pub emission: Compact<u64>,
    pub tao_emission: Compact<u64>,
    pub drain: Compact<u64>,
    pub is_registered: bool,
}
