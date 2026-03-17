import { NextRequest, NextResponse } from "next/server";
import { ethers } from "ethers";
import {
  getBranchSHA, createBranch, createOrUpdateFile,
  createPullRequest, findOpenPR, updatePullRequest,
} from "@/lib/github-pr";
import {
  checkSubmitRateLimit, buildSigningMessage, validateMetadataStructure,
  IDENTITY_CONTRACT_FIELD, getRpcUrl, verifyOnChainOwnership, buildPrTitleAndBody,
} from "@/lib/appchain-registry";
export async function POST(req: NextRequest) {
  try {
    const ip = req.headers.get("x-forwarded-for")?.split(",")[0] || "unknown";
    if (!checkSubmitRateLimit(ip)) {
      return NextResponse.json(
        { success: false, error: "Rate limit exceeded. Max 5 submissions per hour.", code: "RATE_LIMITED" },
        { status: 429 },
      );
    }

    const body = await req.json();
    const { metadata, operation = "register" } = body;
    if (!metadata) {
      return NextResponse.json({ success: false, error: "Missing metadata", code: "INVALID_METADATA" }, { status: 400 });
    }

    // 1. Validate structure
    const structErrors = validateMetadataStructure(metadata);
    if (structErrors.length > 0) {
      return NextResponse.json(
        { success: false, error: `Metadata validation failed: ${structErrors.join(", ")}`, code: "INVALID_METADATA" },
        { status: 400 },
      );
    }

    // 2. Verify signature
    let recoveredAddress: string;
    try {
      const message = buildSigningMessage(metadata, operation);
      recoveredAddress = ethers.verifyMessage(message, metadata.metadata.signature);
    } catch (e) {
      console.error("[appchain-registry] Signature verification failed:", (e as Error).message);
      return NextResponse.json(
        { success: false, error: "Signature verification failed", code: "INVALID_SIGNATURE" },
        { status: 400 },
      );
    }

    if (recoveredAddress.toLowerCase() !== metadata.metadata.signedBy.toLowerCase()) {
      return NextResponse.json(
        { success: false, error: `Recovered signer ${recoveredAddress} does not match signedBy ${metadata.metadata.signedBy}`, code: "INVALID_SIGNATURE" },
        { status: 400 },
      );
    }

    // 3. Check timestamp
    const ts = operation === "register"
      ? Math.floor(new Date(metadata.createdAt).getTime() / 1000)
      : Math.floor(new Date(metadata.lastUpdated).getTime() / 1000);
    if (!Number.isFinite(ts)) {
      return NextResponse.json({ success: false, error: "Invalid timestamp", code: "INVALID_METADATA" }, { status: 400 });
    }
    const now = Math.floor(Date.now() / 1000);
    if (ts > now + 300) {
      return NextResponse.json({ success: false, error: "Timestamp is in the future", code: "SIGNATURE_EXPIRED" }, { status: 400 });
    }
    if (now - ts > 86400) {
      return NextResponse.json({ success: false, error: "Signature expired (>24h)", code: "SIGNATURE_EXPIRED" }, { status: 400 });
    }

    // 4. On-chain ownership check
    const rpcUrl = getRpcUrl(metadata.l1ChainId, metadata.l1RpcUrl);
    if (!rpcUrl) {
      return NextResponse.json(
        { success: false, error: `No L1 RPC available for chain ID ${metadata.l1ChainId}. Include l1RpcUrl in metadata.`, code: "INVALID_METADATA" },
        { status: 400 },
      );
    }

    const identityField = IDENTITY_CONTRACT_FIELD[metadata.stackType];
    const timelockAddress = metadata.l1Contracts[identityField];

    const ownership = await verifyOnChainOwnership(rpcUrl, timelockAddress, recoveredAddress);
    if (!ownership.valid) {
      console.error("[appchain-registry] Ownership check failed:", ownership.error);
      const status = ownership.rpcError ? 502 : 403;
      return NextResponse.json(
        { success: false, error: "On-chain ownership verification failed", code: "OWNERSHIP_CHECK_FAILED" },
        { status },
      );
    }

    // 5. Determine file path
    const filePath = `tokamak-appchain-data/${metadata.l1ChainId}/${metadata.stackType}/${timelockAddress.toLowerCase()}.json`;

    // 6. Check for existing open PR
    const existingPR = await findOpenPR(filePath);

    const fileContent = JSON.stringify(metadata, null, 2) + "\n";
    const commitMsg = operation === "register"
      ? `feat: register ${metadata.name} (${metadata.stackType})`
      : `feat: update ${metadata.name} (${metadata.stackType})`;

    const { prTitle, prBody } = buildPrTitleAndBody(metadata, operation, timelockAddress);

    if (existingPR && existingPR.headBranch) {
      await createOrUpdateFile(filePath, fileContent, existingPR.headBranch, commitMsg);
      await updatePullRequest(existingPR.prNumber, { title: prTitle, body: prBody });

      return NextResponse.json({
        success: true, updated: true,
        prUrl: existingPR.prUrl, prNumber: existingPR.prNumber, filePath,
      });
    }

    // 7. Create new PR
    const branchName = `appchain-registry/${metadata.l1ChainId}/${timelockAddress.toLowerCase().slice(0, 10)}/${ts}`;
    const mainSha = await getBranchSHA("main");
    await createBranch(branchName, mainSha);
    await createOrUpdateFile(filePath, fileContent, branchName, commitMsg);
    const pr = await createPullRequest(prTitle, prBody, branchName);

    return NextResponse.json({
      success: true, prUrl: pr.prUrl, prNumber: pr.prNumber, filePath,
    });
  } catch (e) {
    console.error("[appchain-registry] Error:", e);
    return NextResponse.json(
      { success: false, error: "Internal server error", code: "GITHUB_API_ERROR" },
      { status: 500 },
    );
  }
}
