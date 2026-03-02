"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { storeApi } from "@/lib/api";
import { Program } from "@/lib/types";

export default function Home() {
  const [featured, setFeatured] = useState<Program[]>([]);

  useEffect(() => {
    storeApi.featured().then(setFeatured).catch(() => {});
  }, []);

  return (
    <div className="max-w-7xl mx-auto px-4 py-16">
      {/* Hero */}
      <div className="text-center mb-16">
        <h1 className="text-5xl font-bold text-gray-900 mb-4">
          Guest Program Store
        </h1>
        <p className="text-xl text-gray-600 max-w-2xl mx-auto">
          Create your own circuits and contracts, share them with everyone.
          Build specialized L2s with custom Guest Programs.
        </p>
        <div className="mt-8 flex gap-4 justify-center">
          <Link
            href="/store"
            className="px-6 py-3 bg-blue-600 text-white rounded-lg text-lg font-medium hover:bg-blue-700"
          >
            Explore Store
          </Link>
          <Link
            href="/creator/new"
            className="px-6 py-3 bg-white text-gray-700 border border-gray-300 rounded-lg text-lg font-medium hover:bg-gray-50"
          >
            Create Program
          </Link>
        </div>
      </div>

      {/* How It Works */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-8 mb-16">
        <div className="bg-white rounded-xl p-6 shadow-sm border">
          <div className="w-10 h-10 bg-blue-100 rounded-lg flex items-center justify-center text-blue-600 font-bold text-lg mb-3">
            1
          </div>
          <h3 className="text-lg font-semibold mb-2">Create</h3>
          <p className="text-gray-600">
            Build your contract + circuit as a Guest Program.
            Upload your ELF binary and verification key.
          </p>
        </div>
        <div className="bg-white rounded-xl p-6 shadow-sm border">
          <div className="w-10 h-10 bg-blue-100 rounded-lg flex items-center justify-center text-blue-600 font-bold text-lg mb-3">
            2
          </div>
          <h3 className="text-lg font-semibold mb-2">Share</h3>
          <p className="text-gray-600">
            Publish to the Store. Others can discover and use your program
            to run their own specialized L2.
          </p>
        </div>
        <div className="bg-white rounded-xl p-6 shadow-sm border">
          <div className="w-10 h-10 bg-blue-100 rounded-lg flex items-center justify-center text-blue-600 font-bold text-lg mb-3">
            3
          </div>
          <h3 className="text-lg font-semibold mb-2">Prove</h3>
          <p className="text-gray-600">
            Provers automatically pick up batches. ZK proofs verify
            every state transition on L1.
          </p>
        </div>
      </div>

      {/* Featured Programs */}
      {featured.length > 0 && (
        <div className="mb-16">
          <div className="flex items-center justify-between mb-6">
            <h2 className="text-2xl font-bold">Featured Programs</h2>
            <Link href="/store" className="text-blue-600 hover:underline text-sm">
              View All
            </Link>
          </div>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
            {featured.map((program) => (
              <Link
                key={program.id}
                href={`/store/${program.id}`}
                className="bg-white rounded-xl shadow-sm border p-6 hover:shadow-md transition-shadow"
              >
                <div className="flex items-start gap-4">
                  <div className="w-12 h-12 bg-blue-100 rounded-lg flex items-center justify-center text-blue-600 font-bold text-lg shrink-0">
                    {program.name.charAt(0).toUpperCase()}
                  </div>
                  <div className="min-w-0">
                    <h3 className="font-semibold text-lg truncate">{program.name}</h3>
                    <p className="text-sm text-gray-500 mb-2">{program.program_id}</p>
                    <p className="text-gray-600 text-sm line-clamp-2">
                      {program.description || "No description"}
                    </p>
                  </div>
                </div>
                <div className="flex items-center gap-4 mt-4 pt-4 border-t text-sm text-gray-500">
                  <span className="px-2 py-0.5 bg-gray-100 rounded text-xs">
                    {program.category}
                  </span>
                  <span>{program.use_count} uses</span>
                  {program.is_official && (
                    <span className="px-2 py-0.5 bg-blue-100 text-blue-700 rounded text-xs">
                      Official
                    </span>
                  )}
                </div>
              </Link>
            ))}
          </div>
        </div>
      )}

      {/* Architecture Overview */}
      <div className="bg-gradient-to-br from-gray-50 to-blue-50 rounded-2xl p-8 border">
        <h2 className="text-2xl font-bold mb-6 text-center">Architecture</h2>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-8">
          <div>
            <h3 className="font-semibold text-lg mb-3">Guest Program Modularization</h3>
            <p className="text-gray-600 text-sm mb-4">
              Each Guest Program is an independent circuit that runs inside a zkVM (SP1, RISC0).
              The GuestProgram trait provides a pluggable abstraction so different L2
              applications can use different proving logic.
            </p>
            <ul className="text-sm text-gray-600 space-y-1">
              <li>- EVM-L2: Default Ethereum execution (Type ID 1)</li>
              <li>- ZK-DEX: Decentralized exchange circuits (Type ID 2)</li>
              <li>- Tokamon: Gaming application circuits (Type ID 3)</li>
              <li>- Custom: Build your own (Type ID 10+)</li>
            </ul>
          </div>
          <div>
            <h3 className="font-semibold text-lg mb-3">On-Chain Verification</h3>
            <p className="text-gray-600 text-sm mb-4">
              The GuestProgramRegistry contract on L1 manages program registrations.
              Each program gets a unique programTypeId. The OnChainProposer uses
              a 3D verification key mapping to verify proofs per program type.
            </p>
            <div className="bg-white rounded-lg p-4 font-mono text-xs text-gray-700">
              <p>verificationKeys[commitHash]</p>
              <p className="pl-4">[programTypeId]</p>
              <p className="pl-8">[verifierId] = vk</p>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
