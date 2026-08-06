import { createApi } from "../src/api.js";
import {
  DEV_ACCOUNT_SURIS,
  type DevAccountSuri,
  getAccountFromSuri,
} from "../src/accounts.js";
import { stringifyJson } from "../src/codec.js";
import { uploadContractCode } from "../src/deployment.js";
import { type ContractName } from "../src/constants.js";
import { getOptionalEnv } from "../src/env.js";

const DEFAULT_CONTRACT_UPLOADER: DevAccountSuri = DEV_ACCOUNT_SURIS.alice;
const CONTRACT_UPLOADER_SURI =
  getOptionalEnv("CONTRACT_UPLOADER") ?? DEFAULT_CONTRACT_UPLOADER;

function parseContractName(): ContractName {
  const args = process.argv.slice(2);
  const index = args.indexOf("--contract");
  const value = index === -1 ? undefined : args[index + 1];

  if (
    value !== "erc20" &&
    value !== "auction" &&
    value !== "oracle" &&
    value !== "vault" &&
    value !== "treasury" &&
    value !== "governance" &&
    value !== "election"
  ) {
    throw new Error(
      "Missing or invalid --contract. Expected one of: erc20, auction, oracle, vault, treasury, governance, election.",
    );
  }

  return value;
}

async function main(): Promise<void> {
  const api = await createApi();

  try {
    const contract = parseContractName();
    const contractUploader = await getAccountFromSuri(CONTRACT_UPLOADER_SURI);
    const uploaded = await uploadContractCode(api, contractUploader, contract);
    console.log(
      stringifyJson({
        ...uploaded,
        uploaderAddress: contractUploader.address,
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
