"use client";

import { Suspense, useState, useEffect } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import Link from "next/link";
import { storeApi, deploymentsApi } from "@/lib/api";
import { Program } from "@/lib/types";
import { useAuth } from "@/components/auth-provider";

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

function LaunchPageContent() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { user } = useAuth();

  // Step management
  const [step, setStep] = useState(1);

  // Step 1: Program selection
  const [programs, setPrograms] = useState<Program[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState("");
  const [categories, setCategories] = useState<string[]>([]);
  const [selectedProgram, setSelectedProgram] = useState<Program | null>(null);

  // Step 2: L2 configuration
  const [mode, setMode] = useState<"local" | "production">("local");
  const [l2Name, setL2Name] = useState("");
  const [chainId, setChainId] = useState("");
  const [rpcUrl, setRpcUrl] = useState("");
  const [l1Image, setL1Image] = useState("ethrex");
  const [launching, setLaunching] = useState(false);
  const [error, setError] = useState("");

  // Load programs and categories
  useEffect(() => {
    Promise.all([
      storeApi.programs().catch(() => []),
      storeApi.categories().catch(() => []),
    ]).then(([progs, cats]) => {
      setPrograms(progs);
      setCategories(cats);
      setLoading(false);

      // Deep link: ?program=<id>
      const programId = searchParams.get("program");
      if (programId) {
        const found = progs.find((p: Program) => p.id === programId);
        if (found) {
          setSelectedProgram(found);
          setL2Name(`${found.name} L2`);
          setChainId(generateRandomChainId());
          setStep(2);
        }
      }
    });
  }, [searchParams]);

  // If deep link program not found in list, try fetching directly
  useEffect(() => {
    const programId = searchParams.get("program");
    if (programId && !selectedProgram && !loading) {
      storeApi
        .program(programId)
        .then((p) => {
          setSelectedProgram(p);
          setL2Name(`${p.name} L2`);
          setChainId(generateRandomChainId());
          setStep(2);
        })
        .catch(() => {});
    }
  }, [searchParams, selectedProgram, loading]);

  const generateRandomChainId = () =>
    String(Math.floor(Math.random() * 90000) + 10000);

  const handleSelectProgram = (program: Program) => {
    setSelectedProgram(program);
    setL2Name(`${program.name} L2`);
    if (!chainId) setChainId(generateRandomChainId());
    setError("");
    setStep(2);
  };

  const handleLaunch = async () => {
    if (!selectedProgram) return;
    if (!l2Name.trim()) {
      setError("L2 name is required");
      return;
    }
    setLaunching(true);
    setError("");
    try {
      const deployment = await deploymentsApi.create({
        programId: selectedProgram.id,
        name: l2Name.trim(),
        chainId: chainId ? parseInt(chainId) : undefined,
        rpcUrl: mode === "local" ? undefined : rpcUrl || undefined,
        config: { mode, l1Image: mode === "local" ? l1Image : undefined },
      });
      router.push(`/deployments/${deployment.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to launch L2");
    } finally {
      setLaunching(false);
    }
  };

  // Filter programs
  const filtered = programs.filter((p) => {
    const matchSearch =
      !search ||
      p.name.toLowerCase().includes(search.toLowerCase()) ||
      p.program_id.toLowerCase().includes(search.toLowerCase());
    const matchCategory = !category || p.category === category;
    return matchSearch && matchCategory;
  });

  if (!user) {
    return (
      <div className="max-w-4xl mx-auto px-4 py-16 text-center">
        <h1 className="text-2xl font-bold mb-4">Login Required</h1>
        <p className="text-gray-600 mb-4">You need to be logged in to launch an L2.</p>
        <Link href="/login" className="text-blue-600 hover:underline">
          Go to Login
        </Link>
      </div>
    );
  }

  return (
    <div className="max-w-4xl mx-auto px-4 py-8">
      {/* Step indicator */}
      <div className="flex items-center gap-4 mb-8">
        <div
          className={`flex items-center gap-2 cursor-pointer ${
            step === 1 ? "text-blue-600 font-semibold" : "text-gray-400"
          }`}
          onClick={() => setStep(1)}
        >
          <span
            className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-bold ${
              step === 1
                ? "bg-blue-600 text-white"
                : selectedProgram
                ? "bg-green-100 text-green-700"
                : "bg-gray-200 text-gray-500"
            }`}
          >
            {selectedProgram && step !== 1 ? "\u2713" : "1"}
          </span>
          <span>Select Program</span>
        </div>
        <div className="flex-1 h-px bg-gray-200" />
        <div
          className={`flex items-center gap-2 ${
            step === 2 ? "text-blue-600 font-semibold" : "text-gray-400"
          }`}
        >
          <span
            className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-bold ${
              step === 2 ? "bg-blue-600 text-white" : "bg-gray-200 text-gray-500"
            }`}
          >
            2
          </span>
          <span>Configure & Launch</span>
        </div>
      </div>

      {/* Step 1: Program Selection */}
      {step === 1 && (
        <div>
          <h1 className="text-2xl font-bold mb-2">Select a Guest Program</h1>
          <p className="text-gray-600 mb-6">
            Choose a Guest Program to power your L2 chain.
          </p>

          {/* Search and filter */}
          <div className="flex gap-3 mb-6">
            <input
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search programs..."
              className="flex-1 px-4 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
            />
            <select
              value={category}
              onChange={(e) => setCategory(e.target.value)}
              className="px-4 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
            >
              <option value="">All Categories</option>
              {categories.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
          </div>

          {loading ? (
            <div className="flex justify-center py-16">
              <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600" />
            </div>
          ) : filtered.length === 0 ? (
            <div className="text-center py-16 bg-white rounded-xl border">
              <p className="text-gray-500">No programs found.</p>
            </div>
          ) : (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {filtered.map((program) => (
                <div
                  key={program.id}
                  className="bg-white rounded-xl border p-6 hover:shadow-md transition-shadow"
                >
                  <div className="flex items-start gap-4 mb-4">
                    <div className="w-12 h-12 bg-blue-100 rounded-lg flex items-center justify-center text-blue-600 font-bold text-lg shrink-0">
                      {program.name.charAt(0).toUpperCase()}
                    </div>
                    <div className="min-w-0">
                      <h3 className="font-semibold text-lg truncate">{program.name}</h3>
                      <p className="text-sm text-gray-500">{program.program_id}</p>
                      <div className="flex items-center gap-2 mt-1">
                        <span className="px-2 py-0.5 bg-gray-100 rounded text-xs">
                          {program.category}
                        </span>
                        {program.is_official && (
                          <span className="px-2 py-0.5 bg-blue-100 text-blue-700 rounded text-xs">
                            Official
                          </span>
                        )}
                        <span className="text-xs text-gray-400">{program.use_count} uses</span>
                      </div>
                    </div>
                  </div>
                  <p className="text-gray-600 text-sm mb-4 line-clamp-2">
                    {program.description || "No description"}
                  </p>
                  <button
                    onClick={() => handleSelectProgram(program)}
                    className="w-full px-4 py-2 bg-blue-600 text-white rounded-lg text-sm font-medium hover:bg-blue-700"
                  >
                    Select
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Step 2: Configure & Launch */}
      {step === 2 && selectedProgram && (
        <div>
          <h1 className="text-2xl font-bold mb-2">Configure Your L2</h1>
          <p className="text-gray-600 mb-6">
            Set up your L2 chain powered by{" "}
            <strong>{selectedProgram.name}</strong>.
          </p>

          <div className="bg-white rounded-xl border p-6">
            {/* Selected program info */}
            <div className="flex items-center gap-4 mb-6 pb-6 border-b">
              <div className="w-12 h-12 bg-blue-100 rounded-lg flex items-center justify-center text-blue-600 font-bold text-lg shrink-0">
                {selectedProgram.name.charAt(0).toUpperCase()}
              </div>
              <div>
                <h3 className="font-semibold">{selectedProgram.name}</h3>
                <p className="text-sm text-gray-500">{selectedProgram.program_id}</p>
              </div>
              <button
                onClick={() => setStep(1)}
                className="ml-auto text-sm text-blue-600 hover:underline"
              >
                Change
              </button>
            </div>

            {/* Configuration form */}
            <div className="space-y-4">
              {/* Mode toggle */}
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-2">
                  Environment
                </label>
                <div className="flex gap-2">
                  <button
                    type="button"
                    onClick={() => setMode("local")}
                    className={`flex-1 px-4 py-3 rounded-lg border-2 text-sm font-medium transition-colors ${
                      mode === "local"
                        ? "border-blue-600 bg-blue-50 text-blue-700"
                        : "border-gray-200 text-gray-500 hover:border-gray-300"
                    }`}
                  >
                    <div className="font-semibold">Local</div>
                    <div className="text-xs mt-0.5 font-normal">
                      L1 + L2 both run locally via Docker
                    </div>
                  </button>
                  <button
                    type="button"
                    onClick={() => setMode("production")}
                    className={`flex-1 px-4 py-3 rounded-lg border-2 text-sm font-medium transition-colors ${
                      mode === "production"
                        ? "border-blue-600 bg-blue-50 text-blue-700"
                        : "border-gray-200 text-gray-500 hover:border-gray-300"
                    }`}
                  >
                    <div className="font-semibold">Production</div>
                    <div className="text-xs mt-0.5 font-normal">
                      Connect to an external L1 RPC
                    </div>
                  </button>
                </div>
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  L2 Name *
                </label>
                <input
                  type="text"
                  value={l2Name}
                  onChange={(e) => setL2Name(e.target.value)}
                  className="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                  placeholder="My L2"
                />
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Chain ID
                </label>
                <input
                  type="number"
                  value={chainId}
                  onChange={(e) => setChainId(e.target.value)}
                  className="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                  placeholder="10000~99999 recommended"
                />
                <p className="text-xs text-gray-400 mt-1">
                  {mode === "local"
                    ? "Auto-generated. Any value works for local testing."
                    : "Auto-generated (10000~99999 recommended). For production, use a unique ID not listed on chainlist.org."}
                </p>
              </div>

              {mode === "production" && (
                <div>
                  <label className="block text-sm font-medium text-gray-700 mb-1">
                    L1 RPC URL *
                  </label>
                  <input
                    type="text"
                    value={rpcUrl}
                    onChange={(e) => setRpcUrl(e.target.value)}
                    className="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                    placeholder="https://mainnet.infura.io/v3/..."
                  />
                  <p className="text-xs text-gray-400 mt-1">
                    The Ethereum L1 endpoint your L2 will settle to.
                  </p>
                </div>
              )}

              {mode === "local" && (
                <div className="space-y-3">
                  <div>
                    <label className="block text-sm font-medium text-gray-700 mb-1">
                      L1 Node
                    </label>
                    <select
                      value={l1Image}
                      onChange={(e) => setL1Image(e.target.value)}
                      className="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                    >
                      <option value="ethrex">ethrex (Tokamak)</option>
                      <option value="geth">Geth (go-ethereum)</option>
                      <option value="reth">Reth</option>
                    </select>
                    <p className="text-xs text-gray-400 mt-1">
                      L1 node to run locally alongside your L2.
                    </p>
                  </div>
                  <div className="bg-gray-50 rounded-lg p-4 text-sm text-gray-600">
                    <p className="font-medium text-gray-700 mb-1">Local mode</p>
                    <p>
                      L1 and L2 both run locally via Docker.
                      No external RPC needed — docker-compose will include both services.
                    </p>
                  </div>
                </div>
              )}

              {error && <p className="text-sm text-red-600">{error}</p>}

              <button
                onClick={handleLaunch}
                disabled={launching}
                className="w-full px-6 py-3 bg-blue-600 text-white rounded-lg font-medium hover:bg-blue-700 disabled:opacity-50"
              >
                {launching ? "Launching..." : "Launch L2"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
