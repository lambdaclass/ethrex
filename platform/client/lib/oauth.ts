/**
 * OAuth provider helpers for Google, Naver, Kakao.
 * All secrets come from Vercel Environment Variables.
 */

// --- Google ---
export function isGoogleAuthConfigured(): boolean {
  return !!process.env.GOOGLE_CLIENT_ID;
}

export function getGoogleClientId(): string | null {
  return process.env.GOOGLE_CLIENT_ID || null;
}

export async function verifyGoogleIdToken(idToken: string) {
  // Use Google's tokeninfo endpoint (no SDK dependency needed)
  const res = await fetch(`https://oauth2.googleapis.com/tokeninfo?id_token=${idToken}`);
  if (!res.ok) throw new Error("Invalid Google ID token");

  const payload = await res.json();
  if (payload.aud !== process.env.GOOGLE_CLIENT_ID) {
    throw new Error("Google token audience mismatch");
  }
  if (payload.email_verified !== "true") {
    throw new Error("Google email is not verified");
  }

  return {
    email: payload.email as string,
    name: (payload.name || payload.email.split("@")[0]) as string,
    picture: (payload.picture || null) as string | null,
  };
}

// --- Naver ---
export function isNaverAuthConfigured(): boolean {
  return !!(process.env.NAVER_CLIENT_ID && process.env.NAVER_CLIENT_SECRET);
}

export function getNaverClientId(): string | null {
  return process.env.NAVER_CLIENT_ID || null;
}

export async function exchangeNaverCode(code: string, state: string) {
  const clientId = process.env.NAVER_CLIENT_ID;
  const clientSecret = process.env.NAVER_CLIENT_SECRET;
  if (!clientId || !clientSecret) throw new Error("Naver OAuth is not configured");

  const tokenRes = await fetch(
    "https://nid.naver.com/oauth2.0/token?" +
      new URLSearchParams({
        grant_type: "authorization_code",
        client_id: clientId,
        client_secret: clientSecret,
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
    email: p.email as string,
    name: (p.name || p.nickname || p.email.split("@")[0]) as string,
    picture: (p.profile_image || null) as string | null,
  };
}

// --- Kakao ---
export function isKakaoAuthConfigured(): boolean {
  return !!process.env.KAKAO_REST_API_KEY;
}

export function getKakaoClientId(): string | null {
  return process.env.KAKAO_REST_API_KEY || null;
}

export async function exchangeKakaoCode(code: string, redirectUri: string) {
  const restApiKey = process.env.KAKAO_REST_API_KEY;
  if (!restApiKey) throw new Error("Kakao OAuth is not configured");

  const params: Record<string, string> = {
    grant_type: "authorization_code",
    client_id: restApiKey,
    code,
    redirect_uri: redirectUri,
  };
  if (process.env.KAKAO_CLIENT_SECRET) {
    params.client_secret = process.env.KAKAO_CLIENT_SECRET;
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
    email: (account?.email || `kakao_${profileData.id}@kakao.local`) as string,
    name: (account?.profile?.nickname || `User ${profileData.id}`) as string,
    picture: (account?.profile?.profile_image_url || null) as string | null,
  };
}
