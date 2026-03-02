const { OAuth2Client } = require("google-auth-library");

const GOOGLE_CLIENT_ID = process.env.GOOGLE_CLIENT_ID;

let client = null;

function getClient() {
  if (!GOOGLE_CLIENT_ID) {
    throw new Error("GOOGLE_CLIENT_ID is not configured");
  }
  if (!client) {
    client = new OAuth2Client(GOOGLE_CLIENT_ID);
  }
  return client;
}

async function verifyGoogleIdToken(idToken) {
  const ticket = await getClient().verifyIdToken({
    idToken,
    audience: GOOGLE_CLIENT_ID,
  });
  const payload = ticket.getPayload();

  if (!payload.email_verified) {
    throw new Error("Google email is not verified");
  }

  return {
    email: payload.email,
    name: payload.name || payload.email.split("@")[0],
    picture: payload.picture || null,
  };
}

function isGoogleAuthConfigured() {
  return !!GOOGLE_CLIENT_ID;
}

function getGoogleClientId() {
  return GOOGLE_CLIENT_ID || null;
}

module.exports = { verifyGoogleIdToken, isGoogleAuthConfigured, getGoogleClientId };
