"use client";

import { Suspense, useState, useEffect } from "react";
import Link from "next/link";
import { storeApi } from "@/lib/api";
import { Program } from "@/lib/types";

export default function LaunchPage() {
  return (
    <Suspense
      fallback={
        <div className="min-h-[60vh] flex items-center justify-center">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600" />
        </div>
      }
    >
      <LaunchPageContent />
    </Suspense>
  );
}

const DESKTOP_DOWNLOAD_URL = "https://github.com/tokamak-network/ethrex/releases";

function LaunchPageContent() {
  const [programs, setPrograms] = useState<Program[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    storeApi.programs().then(setPrograms).catch(() => {}).finally(() => setLoading(false));
  }, []);

  return (
    <div className="max-w-4xl mx-auto px-4 py-8">
      {/* Hero */}
      <div className="text-center mb-12">
        <div className="w-16 h-16 mx-auto mb-4 bg-blue-100 rounded-2xl flex items-center justify-center">
          <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-blue-600">
            <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"/>
          </svg>
        </div>
        <h1 className="text-3xl font-bold mb-3">Launch Your Own L2</h1>
        <p className="text-gray-600 text-lg max-w-2xl mx-auto">
          Deploy your own Layer 2 blockchain powered by Tokamak Network.
          Install the Desktop App, create your appchain, and start building in minutes.
        </p>
      </div>

      {/* Steps */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mb-12">
        <div className="bg-white rounded-xl border p-6 text-center">
          <div className="w-10 h-10 mx-auto mb-3 bg-blue-50 rounded-full flex items-center justify-center text-blue-600 font-bold">1</div>
          <h3 className="font-semibold mb-2">Install Desktop App</h3>
          <p className="text-sm text-gray-600">
            Download and install the Tokamak Desktop App. Your control center for appchain management.
          </p>
        </div>
        <div className="bg-white rounded-xl border p-6 text-center">
          <div className="w-10 h-10 mx-auto mb-3 bg-blue-50 rounded-full flex items-center justify-center text-blue-600 font-bold">2</div>
          <h3 className="font-semibold mb-2">Create Your Appchain</h3>
          <p className="text-sm text-gray-600">
            Create your L2 appchain with one click. Choose local dev mode or connect to Sepolia/Ethereum.
          </p>
        </div>
        <div className="bg-white rounded-xl border p-6 text-center">
          <div className="w-10 h-10 mx-auto mb-3 bg-blue-50 rounded-full flex items-center justify-center text-blue-600 font-bold">3</div>
          <h3 className="font-semibold mb-2">Browse Programs & Run</h3>
          <p className="text-sm text-gray-600">
            Explore the Program Store for guest programs, deploy them on your chain, and start building.
          </p>
        </div>
      </div>

      {/* Download CTA */}
      <div className="bg-gradient-to-r from-blue-600 to-indigo-600 rounded-2xl p-8 text-white text-center mb-12">
        <h2 className="text-2xl font-bold mb-3">Get the Tokamak Desktop App</h2>
        <p className="text-blue-100 mb-6 max-w-xl mx-auto">
          The Desktop App is your control center for L2 management.
          Build, deploy, monitor logs, and manage your chain — all from one place.
        </p>
        <div className="flex justify-center gap-4">
          <a
            href={DESKTOP_DOWNLOAD_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-2 px-6 py-3 bg-white text-blue-600 rounded-lg font-semibold hover:bg-blue-50 transition-colors"
          >
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
              <polyline points="7 10 12 15 17 10"/>
              <line x1="12" y1="15" x2="12" y2="3"/>
            </svg>
            Download for macOS
          </a>
          <a
            href={DESKTOP_DOWNLOAD_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-2 px-6 py-3 bg-white/20 text-white rounded-lg font-semibold hover:bg-white/30 transition-colors"
          >
            Download for Linux
          </a>
        </div>
        <p className="text-blue-200 text-sm mt-4">
          macOS and Linux supported
        </p>
      </div>

      {/* What Desktop App can do */}
      <div className="mb-12">
        <h2 className="text-xl font-bold mb-6 text-center">What You Can Do</h2>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {[
            { title: "One-Click Appchain Creation", desc: "Create your L2 appchain instantly. Local dev mode or connect to Sepolia/Ethereum mainnet." },
            { title: "Process Lifecycle Management", desc: "Start, stop, and monitor your appchain processes with one click from the Desktop App." },
            { title: "Real-time Log Viewer", desc: "Watch runtime logs in real-time. Debug and monitor your appchain as it runs." },
            { title: "AI Pilot", desc: "AI-powered assistant that understands your appchain state. Get help with configuration, troubleshooting, and more." },
            { title: "Program Store", desc: "Browse and deploy guest programs on your appchain. Extend functionality with community-built programs." },
            { title: "Open Appchain Registry", desc: "Publish your L2 to the Tokamak Open Appchain registry for others to discover and connect." },
          ].map(({ title, desc }) => (
            <div key={title} className="bg-white rounded-xl border p-5">
              <h3 className="font-semibold mb-1">{title}</h3>
              <p className="text-sm text-gray-600">{desc}</p>
            </div>
          ))}
        </div>
      </div>

      {/* Available Programs */}
      <div className="mb-12">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-xl font-bold">Available Programs</h2>
          <Link href="/store" className="text-sm text-blue-600 hover:underline">
            View all in Store &rarr;
          </Link>
        </div>
        {loading ? (
          <div className="flex justify-center py-8">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600" />
          </div>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
            {programs.slice(0, 6).map((program) => (
              <Link
                key={program.id}
                href={`/store/${program.id}`}
                className="bg-white rounded-xl border p-5 hover:shadow-md transition-shadow"
              >
                <div className="flex items-center gap-3 mb-2">
                  <div className="w-10 h-10 bg-blue-100 rounded-lg flex items-center justify-center text-blue-600 font-bold">
                    {program.name.charAt(0).toUpperCase()}
                  </div>
                  <div>
                    <h3 className="font-semibold text-sm">{program.name}</h3>
                    <p className="text-xs text-gray-500">{program.program_id}</p>
                  </div>
                </div>
                <p className="text-sm text-gray-600 line-clamp-2">{program.description || "No description"}</p>
                <div className="mt-2 flex items-center gap-2">
                  <span className="text-xs px-2 py-0.5 bg-gray-100 rounded">{program.category}</span>
                  <span className="text-xs text-gray-400">{program.use_count} deployments</span>
                </div>
              </Link>
            ))}
          </div>
        )}
      </div>

      {/* FAQ */}
      <div className="mb-8">
        <h2 className="text-xl font-bold mb-4 text-center">FAQ</h2>
        <div className="space-y-3 max-w-2xl mx-auto">
          {[
            { q: "Do I need Docker?", a: "Not for local dev mode. Just install the Desktop App and create an appchain with one click. Docker is optional for advanced deployment scenarios." },
            { q: "Can I run without the Desktop App?", a: "Yes! You can use the CLI directly: `ethrex l2 --dev`. The Desktop App provides a GUI with monitoring, logs, AI Pilot, and lifecycle management." },
            { q: "How much does it cost?", a: "Free. Tokamak Network does not charge for L2 deployment. You only need ETH for L1 gas fees when deploying to testnet or mainnet." },
            { q: "Can I make my L2 public?", a: "Yes. After creating your appchain on testnet/mainnet, you can publish it to the Open Appchain registry from the Desktop App for others to discover." },
          ].map(({ q, a }) => (
            <details key={q} className="bg-white rounded-xl border p-4 group">
              <summary className="font-medium cursor-pointer list-none flex items-center justify-between">
                {q}
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="transition-transform group-open:rotate-180">
                  <polyline points="6 9 12 15 18 9"/>
                </svg>
              </summary>
              <p className="text-sm text-gray-600 mt-2">{a}</p>
            </details>
          ))}
        </div>
      </div>
    </div>
  );
}
