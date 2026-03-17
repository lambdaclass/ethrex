/**
 * Input validation helpers.
 * Ported from platform/server/lib/validate.js
 */

export function sanitizeString(value: unknown, maxLength = 500): string | null {
  if (typeof value !== "string") return null;
  return value.trim().slice(0, maxLength);
}

export function isValidEmail(email: unknown): boolean {
  if (!email || typeof email !== "string") return false;
  const re = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
  return re.test(email) && email.length <= 254;
}

export function isValidProgramId(id: unknown): boolean {
  if (!id || typeof id !== "string") return false;
  return /^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$/.test(id);
}

export function isValidPassword(password: unknown): boolean {
  if (!password || typeof password !== "string") return false;
  return password.length >= 8 && password.length <= 128;
}

export function isValidName(name: unknown): boolean {
  if (!name || typeof name !== "string") return false;
  const trimmed = name.trim();
  return trimmed.length >= 1 && trimmed.length <= 100;
}

export const VALID_CATEGORIES = [
  "general", "defi", "gaming", "payments", "nft", "social", "infrastructure", "other",
] as const;

export function isValidCategory(category: unknown): boolean {
  return VALID_CATEGORIES.includes(category as typeof VALID_CATEGORIES[number]);
}
