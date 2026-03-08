"use client";

import { useState } from "react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useAuth } from "@/components/auth-provider";
import { authApi } from "@/lib/api";
import { NaverLoginButton, KakaoLoginButton, GoogleLoginButton } from "@/components/social-login-buttons";

export default function LoginPage() {
  const { login } = useAuth();
  const searchParams = useSearchParams();
  const desktopCode = searchParams.get("desktop_code");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
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

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setLoading(true);
    try {
      const data = await authApi.login(email, password);
      login(data.token, data.user);
      await linkDesktopCode(data.token);
      if (!desktopCode) window.location.href = "/store";
    } catch (err) {
      setError(err instanceof Error ? err.message : "Login failed");
    } finally {
      setLoading(false);
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

        <div className="space-y-3 mb-6">
          <GoogleLoginButton onDesktopLogin={desktopCode ? linkDesktopCode : undefined} />
          <NaverLoginButton />
          <KakaoLoginButton />
        </div>

        <div className="relative mb-6">
          <div className="absolute inset-0 flex items-center">
            <div className="w-full border-t border-gray-200" />
          </div>
          <div className="relative flex justify-center text-sm">
            <span className="bg-white px-4 text-gray-500">or</span>
          </div>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          {error && (
            <div className="p-3 bg-red-50 text-red-600 text-sm rounded-lg">{error}</div>
          )}
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">Email</label>
            <input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              required
              className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">Password</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
              className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            />
          </div>
          <button
            type="submit"
            disabled={loading}
            className="w-full py-2.5 bg-blue-600 text-white rounded-lg font-medium hover:bg-blue-700 disabled:opacity-50"
          >
            {loading ? "Logging in..." : "Login"}
          </button>
        </form>

        <p className="text-center text-sm text-gray-500 mt-4">
          No account?{" "}
          <Link href="/signup" className="text-blue-600 hover:underline">
            Sign up
          </Link>
        </p>
      </div>
    </div>
  );
}
