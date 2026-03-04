"use client";

import { Suspense, useEffect, useState } from "react";
import { useSearchParams } from "next/navigation";
import { useAuth } from "@/components/auth-provider";
import { authApi } from "@/lib/api";

function KakaoCallbackContent() {
  const searchParams = useSearchParams();
  const { login } = useAuth();
  const [error, setError] = useState("");

  useEffect(() => {
    const code = searchParams.get("code");

    if (!code) {
      setError("Missing authorization code");
      return;
    }

    const redirectUri = `${window.location.origin}/auth/callback/kakao`;

    authApi
      .kakao(code, redirectUri)
      .then((data) => {
        login(data.token, data.user);
        window.location.href = "/store";
      })
      .catch((err) => {
        setError(err instanceof Error ? err.message : "Kakao login failed");
      });
  }, [searchParams, login]);

  if (error) {
    return (
      <div className="min-h-[80vh] flex items-center justify-center">
        <div className="bg-white rounded-xl shadow-sm border p-8 w-full max-w-md text-center">
          <h1 className="text-xl font-bold text-red-600 mb-4">Login Failed</h1>
          <p className="text-gray-600 mb-4">{error}</p>
          <a href="/login" className="text-blue-600 hover:underline">
            Back to Login
          </a>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-[80vh] flex items-center justify-center">
      <div className="text-center">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600 mx-auto mb-4" />
        <p className="text-gray-600">Logging in with Kakao...</p>
      </div>
    </div>
  );
}

export default function KakaoCallbackPage() {
  return (
    <Suspense
      fallback={
        <div className="min-h-[80vh] flex items-center justify-center">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600 mx-auto" />
        </div>
      }
    >
      <KakaoCallbackContent />
    </Suspense>
  );
}
