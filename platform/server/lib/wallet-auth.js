// Lazy-load ethers to avoid ~200ms cold-start penalty on server startup
let _verifyMessage = null;
function getVerifyMessage() {
  if (!_verifyMessage) _verifyMessage = require("ethers").verifyMessage;
  return _verifyMessage;
}

const CHALLENGE_MESSAGE =
  "Sign in to Tokamak Appchain Showroom\n\nDomain: platform.tokamak.network\nPurpose: Social interaction authentication\n\nThis signature proves you own this wallet.";

// LRU cache for verified signatures (avoid repeated ecrecover on same session)
const _verifiedCache = new Map();
const CACHE_MAX = 1000;

/**
 * Verify EVM wallet signature against the challenge message.
 * Returns the recovered address (lowercase) or throws.
 */
function verifyWalletSignature(signature, claimedAddress) {
  const cacheKey = `${signature}:${claimedAddress.toLowerCase()}`;
  if (_verifiedCache.has(cacheKey)) return _verifiedCache.get(cacheKey);

  const recovered = getVerifyMessage()(CHALLENGE_MESSAGE, signature);
  if (recovered.toLowerCase() !== claimedAddress.toLowerCase()) {
    throw new Error("Signature does not match claimed address");
  }
  const addr = recovered.toLowerCase();

  // Evict oldest entries when cache is full
  if (_verifiedCache.size >= CACHE_MAX) {
    const firstKey = _verifiedCache.keys().next().value;
    _verifiedCache.delete(firstKey);
  }
  _verifiedCache.set(cacheKey, addr);

  return addr;
}

/**
 * Express middleware: requires x-wallet-address and x-wallet-signature headers.
 * Sets req.walletAddress on success.
 */
function requireWallet(req, res, next) {
  const address = req.headers["x-wallet-address"];
  const signature = req.headers["x-wallet-signature"];
  if (!address || !signature) {
    return res.status(401).json({ error: "Wallet authentication required" });
  }
  try {
    req.walletAddress = verifyWalletSignature(signature, address);
    next();
  } catch (e) {
    return res.status(401).json({ error: "Invalid wallet signature" });
  }
}

module.exports = { verifyWalletSignature, requireWallet, CHALLENGE_MESSAGE };
