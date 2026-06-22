import type { ApiPromise } from "@polkadot/api";
import type { KeyringPair } from "@polkadot/keyring/types";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import {
  DEV_ACCOUNT_SURIS,
  type DevAccountSuri,
  getAccountFromSuri,
} from "../src/accounts.js";
import { createApi } from "../src/api.js";
import { formatInkError } from "../src/codec.js";
import { assertContractExists } from "../src/contract.js";
import { getOptionalEnv } from "../src/env.js";
import {
  deployOracle,
  dryRunCommitRound,
  dryRunSubmitPrice,
  getOracleContract,
  queryGovernance,
  queryNetuid,
  setNetuid,
  setValidator,
  submitPrice,
} from "../src/interactions/oracle.js";

const DEFAULT_ORACLE_CONTRACT_OWNER: DevAccountSuri = DEV_ACCOUNT_SURIS.alice;
const DEFAULT_ORACLE_REPORTER_1: DevAccountSuri = DEV_ACCOUNT_SURIS.bob;
const DEFAULT_ORACLE_REPORTER_2: DevAccountSuri = DEV_ACCOUNT_SURIS.charlie;
const DEFAULT_ORACLE_REPORTER_3: DevAccountSuri = DEV_ACCOUNT_SURIS.dave;

const ORACLE_CONTRACT_OWNER_SURI =
  getOptionalEnv("ORACLE_CONTRACT_OWNER") ?? DEFAULT_ORACLE_CONTRACT_OWNER;
const ORACLE_REPORTER_1_SURI =
  getOptionalEnv("ORACLE_REPORTER_1") ?? DEFAULT_ORACLE_REPORTER_1;
const ORACLE_REPORTER_2_SURI =
  getOptionalEnv("ORACLE_REPORTER_2") ?? DEFAULT_ORACLE_REPORTER_2;
const ORACLE_REPORTER_3_SURI =
  getOptionalEnv("ORACLE_REPORTER_3") ?? DEFAULT_ORACLE_REPORTER_3;
const ORACLE_CONTRACT_ADDRESS = getOptionalEnv("ORACLE_CONTRACT_ADDRESS");
const ORACLE_NETUID = Number(getOptionalEnv("ORACLE_NETUID") ?? "0");

describe.sequential("tusdt-oracle on-chain flow", () => {
  let api: ApiPromise;
  let governance: KeyringPair;
  let reporters: [KeyringPair, KeyringPair, KeyringPair];
  let oracle: Awaited<ReturnType<typeof deployOracle>>;

  beforeAll(async () => {
    api = await createApi();
    governance = await getAccountFromSuri(ORACLE_CONTRACT_OWNER_SURI);
    reporters = [
      await getAccountFromSuri(ORACLE_REPORTER_1_SURI),
      await getAccountFromSuri(ORACLE_REPORTER_2_SURI),
      await getAccountFromSuri(ORACLE_REPORTER_3_SURI),
    ];
    if (ORACLE_CONTRACT_ADDRESS) {
      await assertContractExists(
        api,
        ORACLE_CONTRACT_ADDRESS,
        "Oracle contract",
      );
      oracle = getOracleContract(api, ORACLE_CONTRACT_ADDRESS);
    } else {
      oracle = await deployOracle(
        api,
        governance,
        governance.address,
        governance.address,
        ORACLE_NETUID,
      );
    }
    console.log("Oracle Address: ", oracle.address.toHuman());
  });

  afterAll(async () => {
    await api.disconnect();
  });

  it("deploys with the configured governance", async () => {
    await expect(queryGovernance(oracle, governance.address)).resolves.toBe(
      governance.address,
    );
  });

  it("deploys with the configured netuid", async () => {
    await expect(queryNetuid(oracle, governance.address)).resolves.toBe(
      ORACLE_NETUID,
    );
  });

  it("lets governance update the netuid", async () => {
    await setNetuid(api, oracle, governance, 42);

    await expect(queryNetuid(oracle, governance.address)).resolves.toBe(42);

    // Restore the original netuid.
    await setNetuid(api, oracle, governance, ORACLE_NETUID);
  });

  it("rejects submissions with an invalid (zero) hotkey", async () => {
    const result = await dryRunSubmitPrice(oracle, governance.address, 10, {
      hotKey: "0x0000000000000000000000000000000000000000000000000000000000000000",
    });

    expect(result.decoded.ok).toBe(false);
    expect(formatInkError(result.decoded.error)).toContain("InvalidHotkey");
  });

  it("rejects zero-price submissions", async () => {
    // Use the reporter's own address as the hotkey for the dry run.
    const result = await dryRunSubmitPrice(oracle, reporters[0].address, 0, {
      hotKey: reporters[0].address,
    });

    expect(result.decoded.ok).toBe(false);
    expect(formatInkError(result.decoded.error)).toContain("InvalidPrice");
  });

  it("blocks round commit below quorum", async () => {
    await setValidator(api, oracle, governance, governance.address);

    // Subnet-based auth requires a chain with the proper chain extension.
    // On a local dev node without subnet registration, these submits will
    // fail with ChainExtensionFailed or NotRegisteredInSubnet. The commit
    // test below verifies the quorum guard independently.
    const commitResult = await dryRunCommitRound(oracle, governance.address);

    expect(commitResult.decoded.ok).toBe(false);
    expect(formatInkError(commitResult.decoded.error)).toContain(
      "NotEnoughSubmissions",
    );
  });
});
