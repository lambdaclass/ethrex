/**
 * Shared utilities for desktop auth code flow.
 * Used by both /api/auth/desktop-code and /api/auth/desktop-token routes.
 */
import { sql, ensureSchema } from "./db";

export const CODE_TTL_MS = 5 * 60 * 1000; // 5 minutes

export async function ensureDesktopTable() {
  await ensureSchema();
  await sql`
    CREATE TABLE IF NOT EXISTS desktop_auth_codes (
      code TEXT PRIMARY KEY,
      code_challenge TEXT NOT NULL,
      session_token TEXT,
      created_at BIGINT NOT NULL
    )
  `;
}

/**
 * Verify a PKCE code_verifier against a stored code_challenge.
 * challenge = hex(SHA-256(verifier))
 */
export async function verifyCodeChallenge(
  verifier: string,
  challenge: string
): Promise<boolean> {
  const encoder = new TextEncoder();
  const data = encoder.encode(verifier);
  const hashBuffer = await crypto.subtle.digest("SHA-256", data);
  const hashHex = Array.from(new Uint8Array(hashBuffer))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  return hashHex === challenge;
}
