// Input validation helpers

/**
 * Validate and sanitize a string field
 */
function sanitizeString(value, maxLength = 500) {
  if (typeof value !== "string") return null;
  return value.trim().slice(0, maxLength);
}

/**
 * Validate email format
 */
function isValidEmail(email) {
  if (!email || typeof email !== "string") return false;
  const re = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
  return re.test(email) && email.length <= 254;
}

/**
 * Validate programId format (lowercase letters, numbers, hyphens)
 */
function isValidProgramId(id) {
  if (!id || typeof id !== "string") return false;
  return /^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$/.test(id);
}

/**
 * Validate password strength
 */
function isValidPassword(password) {
  if (!password || typeof password !== "string") return false;
  return password.length >= 8 && password.length <= 128;
}

/**
 * Validate name
 */
function isValidName(name) {
  if (!name || typeof name !== "string") return false;
  const trimmed = name.trim();
  return trimmed.length >= 1 && trimmed.length <= 100;
}

/**
 * Validate category
 */
const VALID_CATEGORIES = ["general", "defi", "gaming", "payments", "nft", "social", "infrastructure", "other"];

function isValidCategory(category) {
  return VALID_CATEGORIES.includes(category);
}

/**
 * Validate URL format
 */
function isValidUrl(url) {
  if (!url || typeof url !== "string") return false;
  try {
    const parsed = new URL(url);
    return ["http:", "https:"].includes(parsed.protocol);
  } catch {
    return false;
  }
}

module.exports = {
  sanitizeString,
  isValidEmail,
  isValidProgramId,
  isValidPassword,
  isValidName,
  isValidCategory,
  isValidUrl,
  VALID_CATEGORIES,
};
