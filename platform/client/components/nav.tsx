"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useAuth } from "./auth-provider";

function NavLink({ href, children }: { href: string; children: React.ReactNode }) {
  const pathname = usePathname();
  const isActive = pathname === href || pathname.startsWith(href + "/");
  return (
    <Link
      href={href}
      className={`relative ${
        isActive
          ? "text-blue-600 font-medium"
          : "text-gray-600 hover:text-gray-900"
      }`}
    >
      {children}
      {isActive && (
        <span className="absolute left-0 right-0 -bottom-[21px] h-0.5 bg-blue-600" />
      )}
    </Link>
  );
}

export function Nav() {
  const { user, logout } = useAuth();

  return (
    <nav className="border-b border-gray-200 bg-white">
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div className="flex justify-between h-16 items-center">
          <div className="flex items-center gap-8">
            <Link href="/" className="text-xl font-bold text-gray-900">
              Tokamak Appchain
            </Link>
            <div className="flex gap-4">
              <NavLink href="/explore">Explore</NavLink>
              <NavLink href="/store">Store</NavLink>
              <NavLink href="/launch">Launch L2</NavLink>
              {user && (
                <>
                  <NavLink href="/creator">My Apps</NavLink>
                  <NavLink href="/deployments">My L2s</NavLink>
                  <NavLink href="/settings">Settings</NavLink>
                </>
              )}
              {user?.role === "admin" && (
                <NavLink href="/admin">Admin</NavLink>
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
