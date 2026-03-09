// Frame modes per EIP-8141
export const FRAME_MODE_DEFAULT = 0;
export const FRAME_MODE_VERIFY = 1;
export const FRAME_MODE_SENDER = 2;

// Pre-deployed contract addresses (from genesis).
export const SPONSOR_ADDRESS =
  "0x1000000000000000000000000000000000000001";
export const MOCK_ERC20_ADDRESS =
  "0x1000000000000000000000000000000000000002";
export const ACCOUNT_ADDRESS =
  "0x1000000000000000000000000000000000000003";
export const VERIFIER_ADDRESS =
  "0x1000000000000000000000000000000000000004";
export const DEPLOYER_PROXY_ADDRESS =
  "0x4e59b44847b379578588920ca78fbf26c0b4956c";

export interface Frame {
  mode: number; // 0=DEFAULT, 1=VERIFY, 2=SENDER
  target: Uint8Array; // 20-byte address, empty for null (calls tx.sender per spec)
  gasLimit: bigint;
  data: Uint8Array;
}

export interface FrameTxParams {
  chainId: bigint;
  nonce: bigint;
  sender: Uint8Array; // 20-byte address
  frames: Frame[];
  maxPriorityFeePerGas: bigint;
  maxFeePerGas: bigint;
  maxFeePerBlobGas?: bigint;
  blobVersionedHashes?: Uint8Array[];
}

export interface Credential {
  credentialId: string;
  publicKey: { x: string; y: string };
  address: string;
}

export interface WebAuthnMetadata {
  authenticatorData: string; // hex
  clientDataJSON: string;
  challengeIndex: number;
  typeIndex: number;
  userVerificationRequired: boolean;
}

export interface SigHashRequest {
  demoType:
    | "simple-send"
    | "sponsored-send"
    | "batch-ops"
    | "deploy-execute";
  params: SimpleSendParams | SponsoredSendParams | BatchOpsParams | DeployExecuteParams;
}

export interface SimpleSendParams {
  from: string;
  to: string;
  amount: string;
}

export interface SponsoredSendParams {
  from: string;
  to: string;
  amount: string;
  sponsorAddress: string;
}

export interface BatchOpsParams {
  from: string;
  operations: Array<{ to: string; value: string; data: string }>;
}

export interface DeployExecuteParams {
  from: string;
  bytecode: string;
  calldata: string;
}

export interface SimpleSendRequest {
  address: string;
  to: string;
  amount: string;
  signature: { r: string; s: string };
  webauthn: WebAuthnMetadata;
}

export interface SponsoredSendRequest {
  address: string;
  to: string;
  amount: string;
  sponsorAddress: string;
  signature: { r: string; s: string };
  webauthn: WebAuthnMetadata;
}

export interface BatchOpsRequest {
  address: string;
  operations: Array<{ to: string; value: string; data: string }>;
  signature: { r: string; s: string };
  webauthn: WebAuthnMetadata;
}

export interface DeployExecuteRequest {
  address: string;
  bytecode: string;
  calldata: string;
  signature: { r: string; s: string };
  webauthn: WebAuthnMetadata;
}
