/**
 * Keychain API routes for deployer private keys.
 *
 * Keys are registered by the user directly in macOS Keychain Access app
 * under the service name "tokamak-appchain". This server only reads them.
 *
 * Raw private keys are NEVER accepted via API — only read from Keychain
 * at deployment time by the deployment engine (server-side only).
 */

const express = require("express");
const router = express.Router();
const keychain = require("../lib/keychain");

// GET /api/keychain/keys — list all saved key names (account names)
router.get("/keys", (req, res) => {
  try {
    const accounts = keychain.listAccounts();
    res.json({ keys: accounts });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/keychain/keys/:name — check if a key exists and return derived address
router.get("/keys/:name", (req, res) => {
  try {
    const { name } = req.params;
    const exists = keychain.hasSecret(name);
    if (!exists) {
      return res.status(404).json({ error: "Key not found" });
    }
    // Derive Ethereum address for private key entries (skip non-key entries like ai-config, ai-api-key)
    const NON_KEY_ENTRIES = ["ai-config", "ai-api-key", "ai-mode"];
    let address = null;
    if (!NON_KEY_ENTRIES.includes(name)) {
      try {
        const pk = keychain.getSecret(name);
        if (pk) {
          const { ethers } = require("ethers");
          const wallet = new ethers.Wallet(pk);
          address = wallet.address;
        }
      } catch (e) {
        // Not a valid private key — skip silently
      }
    }
    res.json({ name, exists: true, address });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;
