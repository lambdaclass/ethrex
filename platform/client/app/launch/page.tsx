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
          Choose a program from the store, install the Desktop app, and start building in minutes.
        </p>
      </div>

      {/* Steps */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mb-12">
        <div className="bg-white rounded-xl border p-6 text-center">
          <div className="w-10 h-10 mx-auto mb-3 bg-blue-50 rounded-full flex items-center justify-center text-blue-600 font-bold">1</div>
          <h3 className="font-semibold mb-2">Choose a Program</h3>
          <p className="text-sm text-gray-600">
            Browse the Program Store and pick an application to run on your L2 chain.
          </p>
        </div>
        <div className="bg-white rounded-xl border p-6 text-center">
          <div className="w-10 h-10 mx-auto mb-3 bg-blue-50 rounded-full flex items-center justify-center text-blue-600 font-bold">2</div>
          <h3 className="font-semibold mb-2">Install Desktop App</h3>
          <p className="text-sm text-gray-600">
            Download and install the Tokamak Desktop App. It handles deployment, monitoring, and management.
          </p>
        </div>
        <div className="bg-white rounded-xl border p-6 text-center">
          <div className="w-10 h-10 mx-auto mb-3 bg-blue-50 rounded-full flex items-center justify-center text-blue-600 font-bold">3</div>
          <h3 className="font-semibold mb-2">Deploy & Run</h3>
          <p className="text-sm text-gray-600">
            Create your L2 chain with one click. Deploy locally with Docker or to a remote server via SSH.
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
          Requires Docker Desktop for local deployment
        </p>
      </div>

      {/* What Desktop App can do */}
      <div className="mb-12">
        <h2 className="text-xl font-bold mb-6 text-center">What You Can Do</h2>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {[
            { title: "Local Docker Deployment", desc: "Build and run your L2 locally using Docker Compose. Full source build or pre-built images." },
            { title: "Remote SSH Deployment", desc: "Deploy to any server via SSH. Upload configs, pull images, and manage remotely." },
            { title: "Real-time Log Viewer", desc: "Watch build progress and runtime logs in real-time with SSE streaming." },
            { title: "Lifecycle Management", desc: "Start, stop, restart, and destroy deployments with one click." },
            { title: "AI Pilot (Coming Soon)", desc: "AI-powered assistant to help configure, troubleshoot, and optimize your L2." },
            { title: "Open Appchain Registry", desc: "Publish your L2 to the Tokamak Open Appchain registry for others to discover." },
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
            { q: "Do I need Docker?", a: "Yes, for local deployment. Docker Desktop handles all the containers (L1, L2, prover, tools). For remote deployment, the target server needs Docker." },
            { q: "Can I deploy without the Desktop App?", a: "Yes! You can use the CLI directly: `ethrex l2 --dev`. The Desktop App provides a GUI with monitoring, logs, and lifecycle management." },
            { q: "How much does it cost?", a: "Local deployment is free. For remote deployment, you provide your own server. Tokamak Network does not charge for L2 deployment." },
            { q: "Can I make my L2 public?", a: "Yes. After deploying, you can publish your L2 to the Open Appchain registry from the Desktop App. Others can then discover and connect to your chain." },
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
