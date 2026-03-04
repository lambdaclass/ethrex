"use client";

import { useRouter } from "next/navigation";

export default function GuidePage() {
  const router = useRouter();

  return (
    <div className="max-w-3xl mx-auto px-4 py-8">
      <button
        onClick={() => router.back()}
        className="text-blue-600 hover:underline text-sm mb-4 inline-block"
      >
        &larr; Back
      </button>

      <h1 className="text-3xl font-bold mb-2">L2 Launch Guide</h1>
      <p className="text-gray-600 mb-8">
        How to run an ethrex L2 with a guest program.
      </p>

      {/* Prerequisites */}
      <div className="bg-amber-50 border border-amber-200 rounded-xl p-4 mb-6 text-sm">
        <p className="font-semibold text-amber-800 mb-2">Prerequisites</p>
        <ul className="text-amber-700 space-y-1">
          <li>
            <a href="https://www.docker.com/get-started/" target="_blank" rel="noopener noreferrer" className="underline">
              Docker Desktop
            </a> installed and running
          </li>
          <li>Git installed</li>
        </ul>
      </div>

      {/* Steps */}
      <div className="bg-white rounded-xl border p-6 mb-6 space-y-5">
        <div>
          <p className="font-semibold text-gray-800 mb-1">1. Clone ethrex</p>
          <pre className="bg-gray-900 text-gray-100 rounded-lg p-4 text-sm">
{`git clone https://github.com/tokamak-network/ethrex.git
cd ethrex`}
          </pre>
        </div>

        <div>
          <p className="font-semibold text-gray-800 mb-1">2. Run with guest program</p>
          <pre className="bg-gray-900 text-gray-100 rounded-lg p-4 text-sm">
{`# Default (evm-l2)
make -C crates/l2 init-guest-program

# Or specify a program
make -C crates/l2 init-guest-program PROGRAM=zk-dex`}
          </pre>
          <p className="text-xs text-gray-400 mt-1">
            First build takes ~10 minutes (compiles Rust from source).
          </p>
        </div>

        <div>
          <p className="font-semibold text-gray-800 mb-1">3. Check</p>
          <pre className="bg-gray-900 text-gray-100 rounded-lg p-4 text-sm">docker compose -f crates/l2/docker-compose.yaml logs -f</pre>
        </div>

        <div>
          <p className="font-semibold text-gray-800 mb-1">4. Stop</p>
          <pre className="bg-gray-900 text-gray-100 rounded-lg p-4 text-sm">make -C crates/l2 down-guest-program</pre>
        </div>
      </div>

      {/* Endpoints */}
      <div className="bg-white rounded-xl border p-6 mb-6">
        <h2 className="font-bold mb-3">Endpoints</h2>
        <div className="grid grid-cols-2 gap-2 text-sm">
          <span className="text-gray-500">L1 RPC</span>
          <code className="text-xs bg-gray-100 px-1 rounded">http://localhost:8545</code>
          <span className="text-gray-500">L2 RPC</span>
          <code className="text-xs bg-gray-100 px-1 rounded">http://localhost:1729</code>
        </div>
      </div>

      {/* Built-in Programs */}
      <div className="bg-white rounded-xl border p-6 mb-6">
        <h2 className="font-bold mb-3">Built-in Guest Programs</h2>
        <div className="text-sm space-y-2">
          <div className="flex justify-between items-center">
            <div>
              <code className="bg-gray-100 px-1.5 py-0.5 rounded text-xs font-bold">evm-l2</code>
              <span className="text-gray-500 ml-2">Default EVM execution</span>
            </div>
          </div>
          <div className="flex justify-between items-center">
            <div>
              <code className="bg-gray-100 px-1.5 py-0.5 rounded text-xs font-bold">zk-dex</code>
              <span className="text-gray-500 ml-2">DEX order matching circuits</span>
            </div>
          </div>
          <div className="flex justify-between items-center">
            <div>
              <code className="bg-gray-100 px-1.5 py-0.5 rounded text-xs font-bold">tokamon</code>
              <span className="text-gray-500 ml-2">Gaming state transitions</span>
            </div>
          </div>
        </div>
      </div>

      {/* AI block */}
      <div className="bg-gray-900 text-gray-100 rounded-xl p-6">
        <div className="flex items-center gap-2 mb-3">
          <span className="px-2 py-0.5 bg-green-700 text-green-100 rounded text-xs font-medium">
            AI / Automation
          </span>
        </div>
        <pre className="text-sm overflow-x-auto leading-relaxed">
{`git clone https://github.com/tokamak-network/ethrex.git && cd ethrex
make -C crates/l2 init-guest-program PROGRAM=evm-l2
# L1 RPC: http://localhost:8545
# L2 RPC: http://localhost:1729
# Stop: make -C crates/l2 down-guest-program`}
        </pre>
        <p className="text-xs text-gray-400 mt-3">
          Full docs:{" "}
          <a
            href="https://github.com/tokamak-network/ethrex/blob/feat/zk/guest-program-modularization/docs/l2/deployment/gp-store-guide.md"
            target="_blank"
            rel="noopener noreferrer"
            className="text-blue-400 hover:underline"
          >
            docs/l2/deployment/gp-store-guide.md
          </a>
        </p>
      </div>
    </div>
  );
}
