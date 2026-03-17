"use client";

import { Suspense, useEffect, useState } from "react";
import { useSearchParams } from "next/navigation";
import { useAuth } from "@/components/auth-provider";
import { authApi } from "@/lib/api";

function NaverCallbackContent() {
  const searchParams = useSearchParams();
  const { login } = useAuth();
  const [error, setError] = useState("");

  useEffect(() => {
    const code = searchParams.get("code");
    const state = searchParams.get("state");
    const savedState = sessionStorage.getItem("naver_state");

    if (!code || !state) {
      setError("Missing authorization code");
      return;
    }

    if (state !== savedState) {
      setError("Invalid state parameter");
      return;
    }

    sessionStorage.removeItem("naver_state");

    authApi
      .naver(code, state)
      .then((data) => {
        login(data.token, data.user);
        window.location.href = "/store";
      })
      .catch((err) => {
        setError(err instanceof Error ? err.message : "Naver login failed");
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
        <p className="text-gray-600">Logging in with Naver...</p>
      </div>
    </div>
  );
}

export default function NaverCallbackPage() {
  return (
    <Suspense
      fallback={
        <div className="min-h-[80vh] flex items-center justify-center">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600 mx-auto" />
        </div>
      }
    >
      <NaverCallbackContent />
    </Suspense>
  );
}
