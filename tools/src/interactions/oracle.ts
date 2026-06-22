import { ContractPromise } from "@polkadot/api-contract";
import type { ApiPromise } from "@polkadot/api";
import type { KeyringPair } from "@polkadot/keyring/types";

import { loadContractArtifact } from "../artifact.js";
import {
  formatInkError,
  parseIntegerArg,
  pickProperty,
  ratioFromInteger,
  ratioInnerToDisplay,
  toNumber,
  toPrimitive,
} from "../codec.js";
import {
  type ExtrinsicWaitFor,
  queryMessage,
  txMessage,
} from "../contract.js";
import { deployOracle as deployOracleContract } from "../deployment.js";

export interface OracleRoundSummary {
  roundId: number;
  reporterCount: number;
  medianPrice: string | null;
}

export interface OraclePriceData {
  roundId: number;
  price: string | null;
  medianPrice: string | null;
  reporterCount: number;
  committedAt: number;
  wasOverridden: boolean;
}

export interface OraclePriceSubmissionMetadata {
  hotKey: string;
  provider?: string | null;
}

export interface OraclePriceSubmission {
  reporter: string;
  price: string | null;
  metadata: OraclePriceSubmissionMetadata;
}

export function parseCliFlag(name: string): string | undefined {
  const args = process.argv.slice(2);
  const index = args.indexOf(name);
  if (index === -1 || index + 1 >= args.length) {
    return undefined;
  }

  return args[index + 1];
}

export function parseBooleanFlag(value: string | undefined, fallback: boolean): boolean {
  if (value === undefined) {
    return fallback;
  }

  return value.toLowerCase() !== "false";
}

function encodeMetadata(metadata: OraclePriceSubmissionMetadata): object {
  return {
    hot_key: metadata.hotKey,
    provider: metadata.provider ?? null,
  };
}

export async function deployOracle(
  api: ApiPromise,
  signer: KeyringPair,
  controllerAddress = signer.address,
  governanceAddress = signer.address,
  netuid = 0,
  waitFor: ExtrinsicWaitFor = "inBlock",
): Promise<ContractPromise> {
  return deployOracleContract(
    api,
    signer,
    controllerAddress,
    governanceAddress,
    netuid,
    waitFor,
  );
}

export function getOracleContract(api: ApiPromise, address: string): ContractPromise {
  const artifact = loadContractArtifact("oracle");
  return new ContractPromise(api, artifact.abi, address);
}

export async function setNetuid(
  api: ApiPromise,
  oracle: ContractPromise,
  signer: KeyringPair,
  netuid: number,
) {
  return txMessage(api, oracle, "set_netuid", signer, [netuid]);
}

export async function setValidator(
  api: ApiPromise,
  oracle: ContractPromise,
  signer: KeyringPair,
  validator: string | null,
) {
  return txMessage(api, oracle, "set_validator", signer, [validator]);
}

export async function submitPrice(
  api: ApiPromise,
  oracle: ContractPromise,
  signer: KeyringPair,
  priceInteger: bigint | number | string,
  metadata: OraclePriceSubmissionMetadata,
) {
  return txMessage(api, oracle, "submit_price", signer, [
    ratioFromInteger(priceInteger),
    encodeMetadata(metadata),
  ]);
}

export async function commitRound(
  api: ApiPromise,
  oracle: ContractPromise,
  signer: KeyringPair,
  overrideInteger?: bigint | number | string,
) {
  const overrideArg =
    overrideInteger === undefined ? null : ratioFromInteger(overrideInteger);
  return txMessage(api, oracle, "commit_round", signer, [overrideArg]);
}

export async function dryRunSubmitPrice(
  oracle: ContractPromise,
  callerAddress: string,
  priceInteger: bigint | number | string,
  metadata: OraclePriceSubmissionMetadata,
) {
  return queryMessage(oracle, "submit_price", callerAddress, [
    ratioFromInteger(priceInteger),
    encodeMetadata(metadata),
  ]);
}

export async function dryRunCommitRound(
  oracle: ContractPromise,
  callerAddress: string,
  overrideInteger?: bigint | number | string,
) {
  const overrideArg =
    overrideInteger === undefined ? null : ratioFromInteger(overrideInteger);
  return queryMessage(oracle, "commit_round", callerAddress, [overrideArg]);
}

export async function queryGovernance(
  oracle: ContractPromise,
  callerAddress: string,
): Promise<string> {
  const result = await queryMessage(oracle, "governance", callerAddress);
  if (!result.decoded.ok) {
    throw new Error(formatInkError(result.decoded.error));
  }

  return String(result.decoded.value);
}

export async function queryNetuid(
  oracle: ContractPromise,
  callerAddress: string,
): Promise<number> {
  const result = await queryMessage(oracle, "get_netuid", callerAddress);
  if (!result.decoded.ok) {
    throw new Error(formatInkError(result.decoded.error));
  }

  return toNumber(result.decoded.value);
}

export async function queryCurrentRoundId(
  oracle: ContractPromise,
  callerAddress: string,
): Promise<number> {
  const result = await queryMessage(oracle, "current_round_id", callerAddress);
  if (!result.decoded.ok) {
    throw new Error(formatInkError(result.decoded.error));
  }

  return toNumber(result.decoded.value);
}

export async function queryCurrentRoundSummary(
  oracle: ContractPromise,
  callerAddress: string,
): Promise<OracleRoundSummary> {
  const result = await queryMessage(
    oracle,
    "get_current_round_summary",
    callerAddress,
  );
  if (!result.decoded.ok) {
    throw new Error(formatInkError(result.decoded.error));
  }

  const summary = toPrimitive(result.decoded.value);
  return {
    roundId: toNumber(pickProperty(summary, "round_id", "roundId")),
    reporterCount: toNumber(
      pickProperty(summary, "reporter_count", "reporterCount"),
    ),
    medianPrice: ratioInnerToDisplay(
      pickProperty(summary, "median_price", "medianPrice"),
    ),
  };
}

export async function queryLatestPrice(
  oracle: ContractPromise,
  callerAddress: string,
): Promise<OraclePriceData | null> {
  const result = await queryMessage(oracle, "get_latest_price", callerAddress);
  if (!result.decoded.ok) {
    throw new Error(formatInkError(result.decoded.error));
  }

  const maybePrice = result.decoded.value;
  if (maybePrice === null || maybePrice === undefined) {
    return null;
  }

  const priceData = toPrimitive(maybePrice);
  return {
    roundId: toNumber(pickProperty(priceData, "round_id", "roundId")),
    price: ratioInnerToDisplay(pickProperty(priceData, "price")),
    medianPrice: ratioInnerToDisplay(
      pickProperty(priceData, "median_price", "medianPrice"),
    ),
    reporterCount: toNumber(
      pickProperty(priceData, "reporter_count", "reporterCount"),
    ),
    committedAt: toNumber(
      pickProperty(priceData, "committed_at", "committedAt"),
    ),
    wasOverridden: Boolean(
      pickProperty(priceData, "was_overridden", "wasOverridden"),
    ),
  };
}

export async function queryRoundSubmissions(
  oracle: ContractPromise,
  callerAddress: string,
  roundId: number,
): Promise<OraclePriceSubmission[]> {
  const result = await queryMessage(
    oracle,
    "get_round_submissions",
    callerAddress,
    [roundId],
  );
  if (!result.decoded.ok) {
    throw new Error(formatInkError(result.decoded.error));
  }

  return (toPrimitive(result.decoded.value) as unknown[]).map((entry) => {
    const submission = toPrimitive(entry);
    const metadataValue = pickProperty(submission, "metadata");
    const metadataPrimitive = toPrimitive(metadataValue);
    const metadata: OraclePriceSubmissionMetadata = {
      hotKey: String(
        pickProperty(metadataPrimitive, "hot_key", "hotKey"),
      ),
      provider: pickProperty(metadataPrimitive, "provider") ?? undefined,
    };

    return {
      reporter: String(pickProperty(submission, "reporter")),
      price: ratioInnerToDisplay(pickProperty(submission, "price")),
      metadata,
    };
  });
}

export function requireAddress(value: string | undefined): string {
  if (!value) {
    throw new Error("Oracle address is required. Pass --address or set ORACLE_ADDRESS.");
  }

  return value;
}

export function requirePrice(value: string | undefined, label = "price"): bigint {
  if (!value) {
    throw new Error(`Missing ${label}. Pass --price or set ${label.toUpperCase()}.`);
  }

  return parseIntegerArg(value, label);
}
