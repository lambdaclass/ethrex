/**
 * macOS Keychain reader for deployer private keys.
 *
 * Uses the `security` CLI to read from the macOS Keychain.
 * Keys are stored by the user via Keychain Access app under
 * the service name "tokamak-appchain".
 *
 * This module is READ-ONLY — private keys are never written
 * programmatically. Users register them directly in Keychain Access.
 *
 * No native modules required — works via child_process.
 */

const { execSync, execFileSync } = require("child_process");

const SERVICE = "tokamak-appchain";

/**
 * Get a secret from the Keychain.
 * @param {string} account - Key name (set as "Account Name" in Keychain Access)
 * @returns {string|null} Secret value or null if not found
 */
function getSecret(account) {
  try {
    const result = execFileSync("security", [
      "find-generic-password",
      "-a", account,
      "-s", SERVICE,
      "-w",
    ], { stdio: "pipe", encoding: "utf-8" });
    return result.trim();
  } catch {
    return null;
  }
}

/**
 * List all account names stored under our service.
 * @returns {string[]} Array of account names
 */
function listAccounts() {
  try {
    const output = execSync(
      `security dump-keychain`,
      { stdio: "pipe", encoding: "utf-8", maxBuffer: 10 * 1024 * 1024 }
    );
    // Parse keychain dump: find blocks with our service name and extract account
    const accounts = [];
    const blocks = output.split("keychain:");
    for (const block of blocks) {
      if (block.includes(`"svce"<blob>="${SERVICE}"`)) {
        const match = block.match(/"acct"<blob>="([^"]*)"/);
        if (match && match[1]) accounts.push(match[1]);
      }
    }
    return accounts;
  } catch {
    return [];
  }
}

/**
 * Check if a secret exists in the Keychain.
 * @param {string} account - Key name
 * @returns {boolean}
 */
function hasSecret(account) {
  try {
    execFileSync("security", [
      "find-generic-password",
      "-a", account,
      "-s", SERVICE,
    ], { stdio: "pipe" });
    return true;
  } catch {
    return false;
  }
}

module.exports = {
  getSecret,
  listAccounts,
  hasSecret,
  SERVICE,
};
