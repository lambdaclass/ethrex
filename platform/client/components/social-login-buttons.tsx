"use client";

import { GoogleLogin, CredentialResponse } from "@react-oauth/google";
import { useAuth } from "./auth-provider";
import { authApi } from "@/lib/api";

const NAVER_CLIENT_ID = process.env.NEXT_PUBLIC_NAVER_CLIENT_ID;
const KAKAO_CLIENT_ID = process.env.NEXT_PUBLIC_KAKAO_CLIENT_ID;
const GOOGLE_CLIENT_ID = process.env.NEXT_PUBLIC_GOOGLE_CLIENT_ID;

export function NaverLoginButton() {
  if (!NAVER_CLIENT_ID) return null;

  const handleClick = () => {
    const state = Math.random().toString(36).substring(7);
    sessionStorage.setItem("naver_state", state);
    const redirectUri = `${window.location.origin}/auth/callback/naver`;
    const url = `https://nid.naver.com/oauth2.0/authorize?response_type=code&client_id=${NAVER_CLIENT_ID}&redirect_uri=${encodeURIComponent(redirectUri)}&state=${state}`;
    window.location.href = url;
  };

  return (
    <button
      onClick={handleClick}
      className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-[#03C75A] text-white rounded-lg hover:bg-[#02b351] transition-colors font-medium"
    >
      N Naver Login
    </button>
  );
}

export function KakaoLoginButton() {
  if (!KAKAO_CLIENT_ID) return null;

  const handleClick = () => {
    const redirectUri = `${window.location.origin}/auth/callback/kakao`;
    const url = `https://kauth.kakao.com/oauth/authorize?response_type=code&client_id=${KAKAO_CLIENT_ID}&redirect_uri=${encodeURIComponent(redirectUri)}`;
    window.location.href = url;
  };

  return (
    <button
      onClick={handleClick}
      className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-[#FEE500] text-[#3C1E1E] rounded-lg hover:bg-[#fdd800] transition-colors font-medium"
    >
      Kakao Login
    </button>
  );
}

export function GoogleLoginButton() {
  const { login } = useAuth();

  if (!GOOGLE_CLIENT_ID) return null;

  const handleSuccess = async (response: CredentialResponse) => {
    if (!response.credential) return;
    try {
      const data = await authApi.google(response.credential);
      login(data.token, data.user);
      window.location.href = "/store";
    } catch (e) {
      console.error("Google login failed:", e);
    }
  };

  return (
    <div className="flex justify-center">
      <GoogleLogin
        onSuccess={handleSuccess}
        onError={() => console.error("Google login error")}
        width="100%"
        theme="outline"
        size="large"
        text="signin_with"
      />
    </div>
  );
}
