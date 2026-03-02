const KAKAO_REST_API_KEY = process.env.KAKAO_REST_API_KEY;
const KAKAO_CLIENT_SECRET = process.env.KAKAO_CLIENT_SECRET;

async function exchangeKakaoCode(code, redirectUri) {
  if (!KAKAO_REST_API_KEY) {
    throw new Error("Kakao OAuth is not configured");
  }

  const params = {
    grant_type: "authorization_code",
    client_id: KAKAO_REST_API_KEY,
    code,
    redirect_uri: redirectUri,
  };
  if (KAKAO_CLIENT_SECRET) {
    params.client_secret = KAKAO_CLIENT_SECRET;
  }

  const tokenRes = await fetch("https://kauth.kakao.com/oauth/token", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams(params),
  });

  const tokenData = await tokenRes.json();
  if (tokenData.error) {
    throw new Error(tokenData.error_description || "Kakao token exchange failed");
  }

  const profileRes = await fetch("https://kapi.kakao.com/v2/user/me", {
    headers: {
      Authorization: `Bearer ${tokenData.access_token}`,
      "Content-Type": "application/x-www-form-urlencoded;charset=utf-8",
    },
  });

  const profileData = await profileRes.json();
  const account = profileData.kakao_account;

  return {
    email: account?.email || `kakao_${profileData.id}@kakao.local`,
    name: account?.profile?.nickname || `User ${profileData.id}`,
    picture: account?.profile?.profile_image_url || null,
  };
}

function isKakaoAuthConfigured() {
  return !!KAKAO_REST_API_KEY;
}

function getKakaoClientId() {
  return KAKAO_REST_API_KEY || null;
}

module.exports = { exchangeKakaoCode, isKakaoAuthConfigured, getKakaoClientId };
