use super::*;
use ink::codegen::Env as _;

impl TusdtVault {
    /// Checks that no active liquidation auction exists for the given (owner, vault_id) pair.
    ///
    /// Reverts with [`Error::VaultInLiquidation`] if an auction is currently running,
    /// preventing operations (borrow, repay, collateral withdrawal) that are unsafe
    /// while a liquidation auction is in progress.
    pub(crate) fn ensure_not_in_liquidation(&self, owner: AccountId, vault_id: u32) -> Result<()> {
        if self.liquidation_auctions.get((owner, vault_id)).is_some() {
            return Err(Error::VaultInLiquidation);
        }
        Ok(())
    }

    /// Loads the calling account's vault by ID, verifying it exists and is not in liquidation.
    ///
    /// Returns `(owner, vault)` on success. Reverts with [`Error::VaultNotFound`] when
    /// the caller has no vault with the given ID, or [`Error::VaultInLiquidation`] when
    /// a liquidation auction is active. Use [`load_vault`](Self::load_vault) instead for
    /// operations that need to bypass the liquidation check (e.g. liquidation settlement).
    pub(crate) fn load_caller_vault(&self, vault_id: u32) -> Result<(AccountId, Vault)> {
        let caller = self.env().caller();
        let vault = self
            .vaults
            .get((caller, vault_id))
            .ok_or(Error::VaultNotFound)?;
        self.ensure_not_in_liquidation(caller, vault_id)?;
        Ok((caller, vault))
    }

    /// Loads any owner's vault by ID without performing a liquidation check.
    ///
    /// Used for cross-owner operations such as triggering a liquidation auction or
    /// settling an auction result, where the caller is not the vault owner. Reverts
    /// with [`Error::VaultNotFound`] when no vault exists for the given owner and ID.
    pub(crate) fn load_vault(&self, owner: AccountId, vault_id: u32) -> Result<Vault> {
        self.vaults
            .get((owner, vault_id))
            .ok_or(Error::VaultNotFound)
    }

    /// Persists a vault record to storage and syncs the per-owner aggregate debt tracker.
    ///
    /// Computes the debt delta by loading the previous vault state (or zero if no vault
    /// existed yet) and calls [`sync_owner_total_debt`](Self::sync_owner_total_debt) to
    /// keep `owner_total_debt` consistent across create, borrow, repay, and liquidation
    /// settlement operations.
    pub(crate) fn save_vault(
        &mut self,
        owner: AccountId,
        vault_id: u32,
        vault: &Vault,
    ) -> Result<()> {
        let previous_vault_debt = self
            .vaults
            .get((owner, vault_id))
            .map(|stored_vault: Vault| stored_vault.debt_balance)
            .unwrap_or_default();
        self.sync_owner_total_debt(owner, previous_vault_debt, vault.debt_balance)?;
        self.vaults.insert((owner, vault_id), vault);
        Ok(())
    }
}
