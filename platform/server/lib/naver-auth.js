const NAVER_CLIENT_ID = process.env.NAVER_CLIENT_ID;
const NAVER_CLIENT_SECRET = process.env.NAVER_CLIENT_SECRET;

async function exchangeNaverCode(code, state) {
  if (!NAVER_CLIENT_ID || !NAVER_CLIENT_SECRET) {
    throw new Error("Naver OAuth is not configured");
  }

  const tokenRes = await fetch(
    "https://nid.naver.com/oauth2.0/token?" +
      new URLSearchParams({
        grant_type: "authorization_code",
        client_id: NAVER_CLIENT_ID,
        client_secret: NAVER_CLIENT_SECRET,
        code,
        state,
      })
  );

  const tokenData = await tokenRes.json();
  if (tokenData.error) {
    throw new Error(tokenData.error_description || "Naver token exchange failed");
  }

  const profileRes = await fetch("https://openapi.naver.com/v1/nid/me", {
    headers: { Authorization: `Bearer ${tokenData.access_token}` },
  });

  const profileData = await profileRes.json();
  if (profileData.resultcode !== "00") {
    throw new Error("Failed to fetch Naver profile");
  }

  const p = profileData.response;
  return {
    email: p.email,
    name: p.name || p.nickname || p.email.split("@")[0],
    picture: p.profile_image || null,
  };
}

function isNaverAuthConfigured() {
  return !!(NAVER_CLIENT_ID && NAVER_CLIENT_SECRET);
}

function getNaverClientId() {
  return NAVER_CLIENT_ID || null;
}

module.exports = { exchangeNaverCode, isNaverAuthConfigured, getNaverClientId };
