"use client";

import Link from "next/link";
import { useAuth } from "./auth-provider";

export function Nav() {
  const { user, logout } = useAuth();

  return (
    <nav className="border-b border-gray-200 bg-white">
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div className="flex justify-between h-16 items-center">
          <div className="flex items-center gap-8">
            <Link href="/" className="text-xl font-bold text-gray-900">
              GP Store
            </Link>
            <div className="flex gap-4">
              <Link href="/store" className="text-gray-600 hover:text-gray-900">
                Store
              </Link>
              {user && (
                <>
                  <Link href="/creator" className="text-gray-600 hover:text-gray-900">
                    My Programs
                  </Link>
                  <Link href="/deployments" className="text-gray-600 hover:text-gray-900">
                    Deployments
                  </Link>
                </>
              )}
              {user?.role === "admin" && (
                <Link href="/admin" className="text-gray-600 hover:text-gray-900">
                  Admin
                </Link>
              )}
            </div>
          </div>
          <div className="flex items-center gap-4">
            {user ? (
              <>
                <Link href="/profile" className="text-sm text-gray-600 hover:text-gray-900">
                  {user.name}
                </Link>
                <button
                  onClick={logout}
                  className="text-sm text-gray-500 hover:text-gray-700"
                >
                  Logout
                </button>
              </>
            ) : (
              <Link
                href="/login"
                className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm hover:bg-blue-700"
              >
                Login
              </Link>
            )}
          </div>
        </div>
      </div>
    </nav>
  );
}
