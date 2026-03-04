"use client";

import { Suspense, useState, useEffect } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import Link from "next/link";
import { storeApi, deploymentsApi, hostsApi } from "@/lib/api";
import { Program, DeploymentEvent, Host } from "@/lib/types";
import { useAuth } from "@/components/auth-provider";
import DeploymentProgress from "@/components/deployment-progress";
import DirectoryPicker from "@/components/directory-picker";

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

  // Step management (1: select app, 2: configure, 3: deploying)
  const [step, setStep] = useState(1);

  // Step 1: Program selection
  const [programs, setPrograms] = useState<Program[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState("");
  const [categories, setCategories] = useState<string[]>([]);
  const [selectedProgram, setSelectedProgram] = useState<Program | null>(null);

  // Step 2: L2 configuration
  const [mode, setMode] = useState<"local" | "remote" | "manual">("local");
  const [l2Name, setL2Name] = useState("");
  const [chainId, setChainId] = useState("");
  const [rpcUrl, setRpcUrl] = useState("");
  const [l1Image, setL1Image] = useState("ethrex");
  const [deployDir, setDeployDir] = useState("");
  const [showDirPicker, setShowDirPicker] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [error, setError] = useState("");

  // Remote hosts
  const [hosts, setHosts] = useState<Host[]>([]);
  const [selectedHostId, setSelectedHostId] = useState<string>("");
  const [hostsLoading, setHostsLoading] = useState(false);

  // Step 3: Deployment progress
  const [deploymentId, setDeploymentId] = useState<string | null>(null);

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

  // Load remote hosts when switching to remote mode
  useEffect(() => {
    if (mode === "remote" && hosts.length === 0) {
      setHostsLoading(true);
      hostsApi
        .list()
        .then((h: Host[]) => {
          setHosts(h);
          const active = h.filter((host: Host) => host.status === "active");
          if (active.length > 0) setSelectedHostId(active[0].id);
        })
        .catch(() => {})
        .finally(() => setHostsLoading(false));
    }
  }, [mode]); // eslint-disable-line react-hooks/exhaustive-deps

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
    if (mode === "remote" && !selectedHostId) {
      setError("Please select a remote server");
      return;
    }
    setLaunching(true);
    setError("");
    try {
      // Create deployment record
      const deployment = await deploymentsApi.create({
        programId: selectedProgram.id,
        name: l2Name.trim(),
        chainId: chainId ? parseInt(chainId) : undefined,
        rpcUrl: mode === "manual" ? rpcUrl || undefined : undefined,
        config: { mode, l1Image: mode === "local" ? l1Image : undefined, deployDir: deployDir.trim() || undefined },
      });

      if (mode === "local") {
        // Start local Docker deployment and show progress
        setDeploymentId(deployment.id);
        setStep(3);
        await deploymentsApi.provision(deployment.id);
      } else if (mode === "remote") {
        // Start remote Docker deployment and show progress
        setDeploymentId(deployment.id);
        setStep(3);
        await deploymentsApi.provision(deployment.id, selectedHostId);
      } else {
        // Manual mode: just save config and go to detail page
        router.push(`/deployments/${deployment.id}`);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to launch L2");
      setLaunching(false);
    }
  };

  const handleDeploymentComplete = (event: DeploymentEvent) => {
    // Deployment finished successfully
    setLaunching(false);
  };

  const handleDeploymentError = (errorMsg: string) => {
    setError(errorMsg);
    setLaunching(false);
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
        {[
          { num: 1, label: "Select App", complete: !!selectedProgram && step > 1 },
          { num: 2, label: "Configure", complete: step > 2 },
          { num: 3, label: "Deploy", complete: false },
        ].map(({ num, label, complete }, idx) => (
          <div key={num} className="contents">
            {idx > 0 && <div className="flex-1 h-px bg-gray-200" />}
            <div
              className={`flex items-center gap-2 ${
                step >= num ? "cursor-pointer" : ""
              } ${
                step === num ? "text-blue-600 font-semibold" : "text-gray-400"
              }`}
              onClick={() => {
                if (num === 1) setStep(1);
                if (num === 2 && selectedProgram) setStep(2);
              }}
            >
              <span
                className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-bold ${
                  step === num
                    ? "bg-blue-600 text-white"
                    : complete
                    ? "bg-green-100 text-green-700"
                    : "bg-gray-200 text-gray-500"
                }`}
              >
                {complete ? "\u2713" : num}
              </span>
              <span>{label}</span>
            </div>
          </div>
        ))}
      </div>

      {/* Step 1: Program Selection (App Store) */}
      {step === 1 && (
        <div>
          <h1 className="text-2xl font-bold mb-2">Select an App</h1>
          <p className="text-gray-600 mb-6">
            Choose an application to deploy on your L2 chain. Each app has its own circuits and verification contracts.
          </p>

          {/* Search and filter */}
          <div className="flex gap-3 mb-6">
            <input
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search apps..."
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
              <p className="text-gray-500">No apps found.</p>
            </div>
          ) : (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {filtered.map((program) => (
                <div
                  key={program.id}
                  className="bg-white rounded-xl border p-6 hover:shadow-md transition-shadow cursor-pointer"
                  onClick={() => handleSelectProgram(program)}
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
                        <span className="text-xs text-gray-400">{program.use_count} deployments</span>
                      </div>
                    </div>
                  </div>
                  <p className="text-gray-600 text-sm mb-4 line-clamp-2">
                    {program.description || "No description"}
                  </p>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      handleSelectProgram(program);
                    }}
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

            {/* App-specific info */}
            <div className="mb-6 p-4 bg-blue-50 rounded-lg">
              <h4 className="text-sm font-medium text-blue-800 mb-2">App Configuration</h4>
              <div className="text-sm text-blue-700 space-y-1">
                {selectedProgram.program_id === "zk-dex" && (
                  <>
                    <p>ZK Circuits: SP1 (DEX order matching + settlement)</p>
                    <p>Verification: SP1 Verifier Contract</p>
                    <p>Genesis: Custom L2 genesis with DEX pre-deploys</p>
                  </>
                )}
                {selectedProgram.program_id === "evm-l2" && (
                  <>
                    <p>Circuits: Standard EVM execution</p>
                    <p>Verification: Default Verifier Contract</p>
                    <p>Genesis: Standard L2 genesis</p>
                  </>
                )}
                {selectedProgram.program_id === "tokamon" && (
                  <>
                    <p>Circuits: Gaming state transition proofs</p>
                    <p>Verification: Default Verifier Contract</p>
                    <p>Genesis: Standard L2 genesis</p>
                  </>
                )}
                {!["zk-dex", "evm-l2", "tokamon"].includes(selectedProgram.program_id) && (
                  <>
                    <p>Custom guest program: {selectedProgram.program_id}</p>
                    <p>Verification: Default Verifier Contract</p>
                  </>
                )}
              </div>
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
                    <div className="font-semibold">Local (Docker)</div>
                    <div className="text-xs mt-0.5 font-normal">
                      Build and run on this machine
                    </div>
                  </button>
                  <button
                    type="button"
                    onClick={() => setMode("remote")}
                    className={`flex-1 px-4 py-3 rounded-lg border-2 text-sm font-medium transition-colors ${
                      mode === "remote"
                        ? "border-blue-600 bg-blue-50 text-blue-700"
                        : "border-gray-200 text-gray-500 hover:border-gray-300"
                    }`}
                  >
                    <div className="font-semibold">Remote Server</div>
                    <div className="text-xs mt-0.5 font-normal">
                      Deploy to a remote server via SSH
                    </div>
                  </button>
                  <button
                    type="button"
                    onClick={() => setMode("manual")}
                    className={`flex-1 px-4 py-3 rounded-lg border-2 text-sm font-medium transition-colors ${
                      mode === "manual"
                        ? "border-blue-600 bg-blue-50 text-blue-700"
                        : "border-gray-200 text-gray-500 hover:border-gray-300"
                    }`}
                  >
                    <div className="font-semibold">Manual</div>
                    <div className="text-xs mt-0.5 font-normal">
                      Download config and run manually
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
                  Auto-generated. Any unique value works.
                </p>
              </div>

              {(mode === "local" || mode === "remote") && (
                <div>
                  <label className="block text-sm font-medium text-gray-700 mb-1">
                    Deploy Directory
                  </label>
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={deployDir}
                      onChange={(e) => setDeployDir(e.target.value)}
                      className="flex-1 px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                      placeholder="~/.tokamak/deployments/<id> (default)"
                    />
                    <button
                      type="button"
                      onClick={() => setShowDirPicker(true)}
                      className="px-4 py-2 border rounded-lg text-sm hover:bg-gray-50 whitespace-nowrap"
                    >
                      Browse
                    </button>
                  </div>
                  <p className="text-xs text-gray-400 mt-1">
                    docker-compose.yaml will be generated here. Leave empty for default.
                  </p>
                  <DirectoryPicker
                    open={showDirPicker}
                    onClose={() => setShowDirPicker(false)}
                    onSelect={(path) => setDeployDir(path)}
                    initialPath={deployDir}
                  />
                </div>
              )}

              {mode === "manual" && (
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
                  </div>
                  <div className="bg-green-50 rounded-lg p-4 text-sm text-green-800 border border-green-200">
                    <p className="font-medium mb-1">Docker deployment</p>
                    <p>
                      Clicking &quot;Deploy L2&quot; will build Docker images and start L1 + L2 + Prover containers automatically.
                      This may take several minutes on the first run.
                    </p>
                  </div>
                </div>
              )}

              {mode === "remote" && (
                <div className="space-y-3">
                  <div>
                    <label className="block text-sm font-medium text-gray-700 mb-1">
                      Target Server *
                    </label>
                    {hostsLoading ? (
                      <div className="flex items-center gap-2 py-2 text-sm text-gray-400">
                        <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-blue-600" />
                        Loading servers...
                      </div>
                    ) : hosts.length === 0 ? (
                      <div className="border rounded-lg p-4 text-center">
                        <p className="text-sm text-gray-500 mb-2">No remote servers configured.</p>
                        <a
                          href="/settings"
                          className="text-sm text-blue-600 hover:underline"
                        >
                          Add a server in Settings
                        </a>
                      </div>
                    ) : (
                      <select
                        value={selectedHostId}
                        onChange={(e) => setSelectedHostId(e.target.value)}
                        className="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
                      >
                        <option value="">Select a server...</option>
                        {hosts.map((host) => (
                          <option key={host.id} value={host.id}>
                            {host.name} ({host.username}@{host.hostname}:{host.port})
                            {host.status === "active" ? "" : " — not tested"}
                          </option>
                        ))}
                      </select>
                    )}
                  </div>
                  <div className="bg-purple-50 rounded-lg p-4 text-sm text-purple-800 border border-purple-200">
                    <p className="font-medium mb-1">Remote deployment</p>
                    <p>
                      Pre-built Docker images will be pulled on the remote server.
                      The server must have Docker installed and accessible via SSH.
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
                {launching
                  ? "Deploying..."
                  : mode === "manual"
                  ? "Create L2 Config"
                  : "Deploy L2"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Step 3: Deployment Progress */}
      {step === 3 && deploymentId && (
        <div>
          <h1 className="text-2xl font-bold mb-2">Deploying Your L2</h1>
          <p className="text-gray-600 mb-6">
            Your L2 <strong>{l2Name}</strong> powered by{" "}
            <strong>{selectedProgram?.name}</strong> is being deployed...
          </p>

          <div className="bg-white rounded-xl border p-6">
            <DeploymentProgress
              deploymentId={deploymentId}
              eventsUrl={deploymentsApi.eventsUrl(deploymentId)}
              remote={mode === "remote"}
              onComplete={handleDeploymentComplete}
              onError={handleDeploymentError}
            />

            <div className="mt-6 pt-6 border-t flex gap-3">
              <Link
                href={`/deployments/${deploymentId}`}
                className="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm font-medium hover:bg-blue-700"
              >
                Go to Dashboard
              </Link>
              <Link
                href="/deployments"
                className="px-4 py-2 border border-gray-300 rounded-lg text-sm hover:bg-gray-50"
              >
                View All Deployments
              </Link>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
