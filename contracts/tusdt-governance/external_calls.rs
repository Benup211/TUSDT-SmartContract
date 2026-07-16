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

    pub(crate) fn forward_vault_cancel_contract_params_update(
        &mut self,
        netuid: u16,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .cancel_contract_params_update(netuid)
            .map_err(|_| Error::VaultCallFailed)
    }

    pub(crate) fn forward_vault_update_treasury(&mut self, new_treasury: AccountId) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .update_treasury(new_treasury)
            .map_err(|_| Error::VaultCallFailed)
    }

    pub(crate) fn forward_vault_update_platform(&mut self, new_platform: AccountId) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .update_platform(new_platform)
            .map_err(|_| Error::VaultCallFailed)
    }

    pub(crate) fn forward_vault_unpause(&mut self) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault.unpause().map_err(|_| Error::VaultCallFailed)
    }

    pub(crate) fn forward_vault_claim_excess_alpha(&mut self, netuid: u16) -> Result<()> {
        self.ensure_maintainer()?;
        self.vault
            .claim_excess_alpha(netuid)
            .map_err(|_| Error::VaultCallFailed)
    }

    // ----- Vault: council-gated (operational/emergency) -----

    pub(crate) fn forward_vault_pause(&mut self) -> Result<()> {
        self.ensure_council()?;
        self.vault.pause().map_err(|_| Error::VaultCallFailed)
    }

    // ----- Oracle: maintainer-gated (config/risk) -----

    pub(crate) fn forward_oracle_set_validator(
        &mut self,
        validator: Option<AccountId>,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.oracle
            .set_validator(validator)
            .map_err(|_| Error::OracleCallFailed)
    }

    pub(crate) fn forward_oracle_set_max_price_deviation(
        &mut self,
        max_price_deviation: Ratio,
    ) -> Result<()> {
        self.ensure_maintainer()?;
        self.oracle
            .set_max_price_deviation(max_price_deviation)
            .map_err(|_| Error::OracleCallFailed)
    }

    pub(crate) fn forward_oracle_commit_round(&mut self, price: Ratio) -> Result<PriceData> {
        self.ensure_maintainer()?;
        self.oracle
            .commit_round_governance(price)
            .map_err(|_| Error::OracleCallFailed)
    }

    pub(crate) fn forward_oracle_set_netuid(&mut self, netuid: u16) -> Result<()> {
        self.ensure_maintainer()?;
        self.oracle
            .set_netuid(netuid)
            .map_err(|_| Error::OracleCallFailed)
    }

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

    pub(crate) fn forward_auction_set_admin(&mut self, admin: Option<AccountId>) -> Result<()> {
        self.ensure_maintainer()?;
        self.auction
            .set_admin(admin)
            .map_err(|_| Error::AuctionCallFailed)
    }
}
