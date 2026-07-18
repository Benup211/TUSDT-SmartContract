// Cross-contract forwarders: the governance contract drives the vault, auction, and oracle by
// calling their `governance`-gated messages. After deployment those contracts' `governance` role
// is this contract, so to them the caller of each forwarded call is `governance` and their own
// `ensure_governance()` checks pass unchanged — no changes are needed on the callee side.
//
// Authorization is decided *here*, inside governance: `ensure_maintainer` for governing/config
// actions, `ensure_council` for the operational/emergency ones. Each helper performs the role
// check and then the typed cross-contract call, mapping the callee's error onto a local
// `*CallFailed` variant.

use super::*;

impl TusdtGovernance {
    // ----- Vault: maintainer-gated (governing/config) -----

    /// Schedules a vault contract-parameter update for a specific netuid.
    /// Maintainer-only. Delegates to `vault.set_contract_params(netuid, params)`.
    pub(crate) fn forward_vault_set_contract_params(
        &mut self,
        netuid: u16,
        params: VaultContractParamsConfig,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .set_contract_params(netuid, params)
            .map_err(|_| Error::VaultCallFailed)
    }

    /// Cancels the vault's scheduled contract-parameter update for a netuid.
    /// Maintainer-only. Delegates to `vault.cancel_contract_params_update(netuid)`.
    pub(crate) fn forward_vault_cancel_contract_params_update(
        &mut self,
        netuid: u16,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .cancel_contract_params_update(netuid)
            .map_err(|_| Error::VaultCallFailed)
    }

    /// Updates the vault's treasury (fee recipient) account.
    /// Maintainer-only. Delegates to `vault.update_treasury(new_treasury)`.
    pub(crate) fn forward_vault_update_treasury(&mut self, new_treasury: AccountId) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .update_treasury(new_treasury)
            .map_err(|_| Error::VaultCallFailed)
    }

    /// Updates the vault's platform (pause operator) account.
    /// Maintainer-only. Delegates to `vault.update_platform(new_platform)`.
    pub(crate) fn forward_vault_update_platform(&mut self, new_platform: AccountId) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .update_platform(new_platform)
            .map_err(|_| Error::VaultCallFailed)
    }

    /// Unpauses the vault; maintainer-only (deliberate recovery).
    /// Delegates to `vault.unpause()`.
    pub(crate) fn forward_vault_unpause(&mut self) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault.unpause().map_err(|_| Error::VaultCallFailed)
    }

    /// Claims excess alpha on a subnet from the vault and sends the TAO to treasury.
    /// Maintainer-only. Delegates to `vault.claim_excess_alpha(netuid)`.
    pub(crate) fn forward_vault_claim_excess_alpha(&mut self, netuid: u16) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .claim_excess_alpha(netuid)
            .map_err(|_| Error::VaultCallFailed)
    }

    /// Approves or removes a netuid for vault alpha collateral.
    /// Maintainer-only. Delegates to `vault.set_approved_netuid(netuid, approved)`.
    pub(crate) fn forward_vault_set_approved_netuid(
        &mut self,
        netuid: u16,
        approved: bool,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .set_approved_netuid(netuid, approved)
            .map_err(|_| Error::VaultCallFailed)
    }

    /// Schedules a vault global-parameter update (24h timelock).
    /// Maintainer-only. Delegates to `vault.set_global_params(config)`.
    pub(crate) fn forward_vault_set_global_params(
        &mut self,
        config: VaultGlobalParamsConfig,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .set_global_params(config)
            .map_err(|_| Error::VaultCallFailed)
    }

    /// Cancels the vault's currently scheduled global-parameter update.
    /// Maintainer-only. Delegates to `vault.cancel_global_params_update()`.
    pub(crate) fn forward_vault_cancel_global_params_update(&mut self) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .cancel_global_params_update()
            .map_err(|_| Error::VaultCallFailed)
    }

    // ----- Vault: council-gated (operational/emergency) -----

    /// Pauses the vault; council-only (operational/emergency halt).
    /// Any single council member can trigger this — no consensus needed.
    /// Delegates to `vault.pause()`.
    pub(crate) fn forward_vault_pause(&mut self) -> Result<()> {
        self.ensure_council()?;
        self.vault.pause().map_err(|_| Error::VaultCallFailed)
    }

    // ----- Vault: upgrade/migration (maintainer-gated) -----

    /// Transfers the ERC20 token controller from the current vault to a new account.
    /// Maintainer-only. Delegates to `vault.set_token_controller(new_controller)`.
    pub(crate) fn forward_vault_set_token_controller(
        &mut self,
        new_controller: AccountId,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .set_token_controller(new_controller)
            .map_err(|_| Error::VaultTokenCallFailed)
    }

    /// Updates the vault's stored auction contract address. Maintainer-only.
    /// Delegates to `vault.update_auction_address(new_auction)`.
    pub(crate) fn forward_vault_update_auction_address(
        &mut self,
        new_auction: AccountId,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .update_auction_address(new_auction)
            .map_err(|_| Error::VaultCallFailed)
    }

    /// Updates the vault's stored oracle contract address. Maintainer-only.
    /// Delegates to `vault.update_oracle_address(new_oracle)`.
    pub(crate) fn forward_vault_update_oracle_address(
        &mut self,
        new_oracle: AccountId,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .update_oracle_address(new_oracle)
            .map_err(|_| Error::VaultCallFailed)
    }

    // ----- Oracle: maintainer-gated (config/risk) -----

    /// Sets/clears the oracle's round-committing validator.
    /// Maintainer-only. Delegates to `oracle.set_validator(validator)`.
    pub(crate) fn forward_oracle_set_validator(
        &mut self,
        validator: Option<AccountId>,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.oracle
            .set_validator(validator)
            .map_err(|_| Error::OracleCallFailed)
    }

    /// Updates the oracle's max allowed price deviation (as a `Ratio`).
    /// Maintainer-only. Delegates to `oracle.set_max_price_deviation(max_price_deviation)`.
    pub(crate) fn forward_oracle_set_max_price_deviation(
        &mut self,
        max_price_deviation: Ratio,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.oracle
            .set_max_price_deviation(max_price_deviation)
            .map_err(|_| Error::OracleCallFailed)
    }

    /// Commits an emergency oracle price, bypassing quorum and deviation checks.
    /// Maintainer-only. Delegates to `oracle.commit_round_governance(price)`.
    pub(crate) fn forward_oracle_commit_round(&mut self, price: Ratio) -> Result<PriceData> {
        self.ensure_maintainer()?;
        self.oracle
            .commit_round_governance(price)
            .map_err(|_| Error::OracleCallFailed)
    }

    /// Updates the governing subnet netuid for the oracle.
    /// Maintainer-only. Delegates to `oracle.set_netuid(netuid)`.
    pub(crate) fn forward_oracle_set_netuid(&mut self, netuid: u16) -> Result<()> {
        self.ensure_maintainer()?;
        self.oracle
            .set_netuid(netuid)
            .map_err(|_| Error::OracleCallFailed)
    }

    /// Updates the minimum submitter stake threshold for the oracle.
    /// Maintainer-only. Delegates to `oracle.set_min_submitter_stake(min_stake)`.
    pub(crate) fn forward_oracle_set_min_submitter_stake(
        &mut self,
        min_stake: u128,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.oracle
            .set_min_submitter_stake(min_stake)
            .map_err(|_| Error::OracleCallFailed)
    }

    // ----- Auction: maintainer-gated (config) -----

    /// Sets/clears the auction admin (allowed to bid on expired no-bid auctions).
    /// Maintainer-only. Delegates to `auction.set_admin(admin)`.
    pub(crate) fn forward_auction_set_admin(&mut self, admin: Option<AccountId>) -> Result<()> {
        self.ensure_maintainer()?;
        self.auction
            .set_admin(admin)
            .map_err(|_| Error::AuctionCallFailed)
    }
}
