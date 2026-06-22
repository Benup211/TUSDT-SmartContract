import { createApi } from "../src/api.js";
import {
  DEV_ACCOUNT_SURIS,
  type DevAccountSuri,
  getAccountFromSuri,
} from "../src/accounts.js";
import { stringifyJson } from "../src/codec.js";
import { deployVault } from "../src/deployment.js";
import { getOptionalEnv } from "../src/env.js";

const DEFAULT_CONTRACT_DEPLOYER: DevAccountSuri = DEV_ACCOUNT_SURIS.alice;
const CONTRACT_DEPLOYER_SURI =
  getOptionalEnv("CONTRACT_DEPLOYER") ?? DEFAULT_CONTRACT_DEPLOYER;

function getCliFlag(name: string): string | undefined {
  const args = process.argv.slice(2);
  const index = args.indexOf(name);
  return index === -1 ? undefined : args[index + 1];
}

function requireValue(value: string | undefined, label: string): string {
  if (!value) {
    throw new Error(`Missing ${label}. Pass it as an env var or CLI flag.`);
  }

  return value;
}

async function main(): Promise<void> {
  const api = await createApi();

  try {
    const contractDeployer = await getAccountFromSuri(CONTRACT_DEPLOYER_SURI);
    const tokenCodeHash = requireValue(
      getCliFlag("--token-code-hash") ??
        getOptionalEnv("VAULT_TOKEN_CODE_HASH"),
      "VAULT_TOKEN_CODE_HASH / --token-code-hash",
    );
    const auctionCodeHash = requireValue(
      getCliFlag("--auction-code-hash") ??
        getOptionalEnv("VAULT_AUCTION_CODE_HASH"),
      "VAULT_AUCTION_CODE_HASH / --auction-code-hash",
    );
    const oracleCodeHash = requireValue(
      getCliFlag("--oracle-code-hash") ??
        getOptionalEnv("VAULT_ORACLE_CODE_HASH"),
      "VAULT_ORACLE_CODE_HASH / --oracle-code-hash",
    );
    const treasuryAddress = requireValue(
      getCliFlag("--treasury-address") ??
        getOptionalEnv("VAULT_TREASURY_ADDRESS"),
      "VAULT_TREASURY_ADDRESS / --treasury-address",
    );
    const netuid = Number(
      requireValue(
        getCliFlag("--netuid") ??
          getOptionalEnv("VAULT_NETUID"),
        "VAULT_NETUID / --netuid",
      ),
    );

    const contract = await deployVault(
      api,
      contractDeployer,
      treasuryAddress,
      tokenCodeHash,
      auctionCodeHash,
      oracleCodeHash,
      netuid,
    );

    console.log(
      stringifyJson({
        address: contract.address.toString(),
        deployerAddress: contractDeployer.address,
        treasuryAddress,
        tokenCodeHash,
        auctionCodeHash,
        oracleCodeHash,
        netuid,
      }),
    );
  } finally {
    await api.disconnect();
  }
}

void main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
