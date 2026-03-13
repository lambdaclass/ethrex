"use client";

import { Suspense, useState } from "react";
import { useSearchParams } from "next/navigation";
import { GoogleLoginButton, NaverLoginButton, KakaoLoginButton } from "@/components/social-login-buttons";

export default function LoginPage() {
  return (
    <Suspense fallback={
      <div className="min-h-[80vh] flex items-center justify-center">
        <div className="text-gray-400">Loading...</div>
      </div>
    }>
      <LoginContent />
    </Suspense>
  );
}

function LoginContent() {
  const searchParams = useSearchParams();
  const desktopCode = searchParams.get("desktop_code");
  const [desktopDone, setDesktopDone] = useState(false);

  const linkDesktopCode = async (token: string) => {
    if (!desktopCode) return;
    try {
      await fetch("/api/auth/desktop-code", {
        method: "PUT",
        headers: {
          "Content-Type": "application/json",
          "Authorization": `Bearer ${token}`,
        },
        body: JSON.stringify({ code: desktopCode }),
      });
      setDesktopDone(true);
    } catch (e) {
      console.error("Failed to link desktop code:", e);
    }
  };

  if (desktopDone) {
    return (
      <div className="min-h-[80vh] flex items-center justify-center">
        <div className="bg-white rounded-xl shadow-sm border p-8 w-full max-w-md text-center space-y-4">
          <div className="text-4xl">&#x2705;</div>
          <h1 className="text-xl font-bold">Desktop App Connected</h1>
          <p className="text-gray-500 text-sm">
            Login successful! You can now close this page and return to the desktop app.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-[80vh] flex items-center justify-center">
      <div className="bg-white rounded-xl shadow-sm border p-8 w-full max-w-md">
        <h1 className="text-2xl font-bold text-center mb-6">Login</h1>

        {desktopCode && (
          <div className="mb-4 p-3 bg-blue-50 text-blue-700 text-sm rounded-lg text-center">
            Desktop app login — sign in to connect
          </div>
        )}

        <div className="space-y-3">
          <GoogleLoginButton onDesktopLogin={desktopCode ? linkDesktopCode : undefined} />
          <NaverLoginButton />
          <KakaoLoginButton />
        </div>
      </div>
    </div>
  );
}
