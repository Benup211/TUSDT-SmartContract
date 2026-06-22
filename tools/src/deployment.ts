import { ContractPromise } from "@polkadot/api-contract";
import type { ApiPromise } from "@polkadot/api";
import type { KeyringPair } from "@polkadot/keyring/types";

import { loadContractArtifact, type ContractArtifact } from "./artifact.js";
import { type ContractName } from "./constants.js";
import {
  type ExtrinsicWaitFor,
  instantiateContract,
  submitExtrinsic,
} from "./contract.js";

export interface UploadedCode {
  contract: ContractName;
  codeHash: string;
  sourceHash: string;
  artifactPath: string;
  blockHash: string;
}

export function getContractInstance(
  api: ApiPromise,
  artifact: ContractArtifact,
  address: string,
): ContractPromise {
  return new ContractPromise(api, artifact.abi, address);
}

export async function uploadContractCode(
  api: ApiPromise,
  signer: KeyringPair,
  contract: ContractName,
): Promise<UploadedCode> {
  const artifact = loadContractArtifact(contract);
  const uploadCode = (
    api.tx.contracts as unknown as {
      uploadCode?: (...args: unknown[]) => {
        signAndSend: ReturnType<typeof submitExtrinsic> extends never
          ? never
          : any;
      };
    }
  ).uploadCode;

  if (!uploadCode) {
    throw new Error(
      "The connected runtime does not expose contracts.uploadCode",
    );
  }

  const argumentCount =
    (uploadCode as unknown as { meta?: { args?: unknown[] } }).meta?.args
      ?.length ?? 0;

  const tx =
    argumentCount >= 3
      ? uploadCode(artifact.wasm, null, "Enforced")
      : uploadCode(artifact.wasm, null);

  const submitted = await submitExtrinsic(api, signer, tx);
  const storedEvent = submitted.events.find(({ event }) =>
    api.events.contracts.CodeStored.is(event),
  );

  return {
    contract,
    codeHash: storedEvent?.event.data[0]?.toString() ?? artifact.sourceHash,
    sourceHash: artifact.sourceHash,
    artifactPath: artifact.artifactPath,
    blockHash: submitted.blockHash,
  };
}

export async function deployErc20(
  api: ApiPromise,
  signer: KeyringPair,
  ownerAddress = signer.address,
  waitFor: ExtrinsicWaitFor = "inBlock",
): Promise<ContractPromise> {
  const artifact = loadContractArtifact("erc20");
  const deployed = await instantiateContract(
    api,
    signer,
    artifact.abi,
    artifact.wasm,
    "new",
    [ownerAddress],
    waitFor,
  );

  return getContractInstance(api, artifact, deployed.address);
}

export async function deployAuction(
  api: ApiPromise,
  signer: KeyringPair,
  tokenAddress: string,
  controllerAddress = signer.address,
  governanceAddress = signer.address,
  waitFor: ExtrinsicWaitFor = "inBlock",
): Promise<ContractPromise> {
  const artifact = loadContractArtifact("auction");
  const deployed = await instantiateContract(
    api,
    signer,
    artifact.abi,
    artifact.wasm,
    "new",
    [controllerAddress, governanceAddress, tokenAddress],
    waitFor,
  );

  return getContractInstance(api, artifact, deployed.address);
}

export async function deployOracle(
  api: ApiPromise,
  signer: KeyringPair,
  controllerAddress = signer.address,
  governanceAddress = signer.address,
  netuid = 0,
  waitFor: ExtrinsicWaitFor = "inBlock",
): Promise<ContractPromise> {
  const artifact = loadContractArtifact("oracle");
  const deployed = await instantiateContract(
    api,
    signer,
    artifact.abi,
    artifact.wasm,
    "new",
    [controllerAddress, governanceAddress, netuid],
    waitFor,
  );

  return getContractInstance(api, artifact, deployed.address);
}

export async function deployVault(
  api: ApiPromise,
  signer: KeyringPair,
  treasuryAddress: string,
  tokenCodeHash: string,
  auctionCodeHash: string,
  oracleCodeHash: string,
  netuid: number,
  waitFor: ExtrinsicWaitFor = "inBlock",
): Promise<ContractPromise> {
  const artifact = loadContractArtifact("vault");
  const deployed = await instantiateContract(
    api,
    signer,
    artifact.abi,
    artifact.wasm,
    "new",
    [treasuryAddress, tokenCodeHash, auctionCodeHash, oracleCodeHash, netuid],
    waitFor,
  );

  return getContractInstance(api, artifact, deployed.address);
}

export async function deployTreasury(
  api: ApiPromise,
  signer: KeyringPair,
  tokenAddress: string,
  waitFor: ExtrinsicWaitFor = "inBlock",
): Promise<ContractPromise> {
  const artifact = loadContractArtifact("treasury");
  const deployed = await instantiateContract(
    api,
    signer,
    artifact.abi,
    artifact.wasm,
    "new",
    [tokenAddress],
    waitFor,
  );

  return getContractInstance(api, artifact, deployed.address);
}

export async function deployGovernance(
  api: ApiPromise,
  signer: KeyringPair,
  treasuryAddress: string,
  vaultAddress: string,
  auctionAddress: string,
  oracleAddress: string,
  maintainer: string,
  electionCodeHash: string,
  waitFor: ExtrinsicWaitFor = "inBlock",
): Promise<ContractPromise> {
  const artifact = loadContractArtifact("governance");
  const deployed = await instantiateContract(
    api,
    signer,
    artifact.abi,
    artifact.wasm,
    "new",
    [treasuryAddress, vaultAddress, auctionAddress, oracleAddress, maintainer, electionCodeHash],
    waitFor,
  );

  return getContractInstance(api, artifact, deployed.address);
}
