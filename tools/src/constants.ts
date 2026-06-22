import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const CURRENT_DIR = dirname(fileURLToPath(import.meta.url));

export const TOOLS_ROOT = resolve(CURRENT_DIR, "..");
export const REPO_ROOT = resolve(TOOLS_ROOT, "..");

export const DEFAULT_WS_URL = "ws://127.0.0.1:9944";
export const RATIO_SCALE = 10n ** 18n;

export const CONTRACT_CONFIGS = {
  erc20: {
    artifactPath: resolve(REPO_ROOT, "target/ink/tusdt_erc20/tusdt_erc20.contract"),
    manifestPath: resolve(REPO_ROOT, "contracts/tusdt-erc20/Cargo.toml"),
  },
  auction: {
    artifactPath: resolve(REPO_ROOT, "target/ink/tusdt_auction/tusdt_auction.contract"),
    manifestPath: resolve(REPO_ROOT, "contracts/tusdt-auction/Cargo.toml"),
  },
  oracle: {
    artifactPath: resolve(REPO_ROOT, "target/ink/tusdt_oracle/tusdt_oracle.contract"),
    manifestPath: resolve(REPO_ROOT, "contracts/tusdt-oracle/Cargo.toml"),
  },
  vault: {
    artifactPath: resolve(REPO_ROOT, "target/ink/tusdt_vault/tusdt_vault.contract"),
    manifestPath: resolve(REPO_ROOT, "contracts/tusdt-vault/Cargo.toml"),
  },
  treasury: {
    artifactPath: resolve(REPO_ROOT, "target/ink/tusdt_treasury/tusdt_treasury.contract"),
    manifestPath: resolve(REPO_ROOT, "contracts/tusdt-treasury/Cargo.toml"),
  },
  governance: {
    artifactPath: resolve(REPO_ROOT, "target/ink/tusdt_governance/tusdt_governance.contract"),
    manifestPath: resolve(REPO_ROOT, "contracts/tusdt-governance/Cargo.toml"),
  },
  election: {
    artifactPath: resolve(REPO_ROOT, "target/ink/tusdt_election/tusdt_election.contract"),
    manifestPath: resolve(REPO_ROOT, "contracts/tusdt-election/Cargo.toml"),
  },
} as const;

export type ContractName = keyof typeof CONTRACT_CONFIGS;
