"use client";

import { useState, useEffect } from "react";
import Link from "next/link";
import { storeApi } from "@/lib/api";
import { Program } from "@/lib/types";

export default function StorePage() {
  const [programs, setPrograms] = useState<Program[]>([]);
  const [categories, setCategories] = useState<string[]>([]);
  const [selectedCategory, setSelectedCategory] = useState("");
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    storeApi.categories().then((data) => setCategories(data)).catch(() => {});
  }, []);

  useEffect(() => {
    setLoading(true);
    const params: Record<string, string> = {};
    if (selectedCategory) params.category = selectedCategory;
    if (search) params.search = search;

    storeApi
      .programs(params)
      .then((data) => setPrograms(data))
      .catch(() => setPrograms([]))
      .finally(() => setLoading(false));
  }, [selectedCategory, search]);

  return (
    <div className="max-w-7xl mx-auto px-4 py-8">
      <div className="flex items-center justify-between mb-8">
        <h1 className="text-3xl font-bold">Guest Program Store</h1>
        <Link
          href="/creator/new"
          className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 font-medium"
        >
          + Create Program
        </Link>
      </div>

      <div className="flex gap-4 mb-6">
        <input
          type="text"
          placeholder="Search programs..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="flex-1 px-4 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-transparent"
        />
        <select
          value={selectedCategory}
          onChange={(e) => setSelectedCategory(e.target.value)}
          className="px-4 py-2 border border-gray-300 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-transparent"
        >
          <option value="">All Categories</option>
          {categories.map((cat) => (
            <option key={cat} value={cat}>
              {cat}
            </option>
          ))}
        </select>
      </div>

      {loading ? (
        <div className="text-center py-16">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600 mx-auto mb-4" />
          <p className="text-gray-500">Loading programs...</p>
        </div>
      ) : programs.length === 0 ? (
        <div className="text-center py-16">
          <p className="text-gray-500 text-lg mb-2">No programs found</p>
          <p className="text-gray-400">
            Be the first to{" "}
            <Link href="/creator/new" className="text-blue-600 hover:underline">
              create a Guest Program
            </Link>
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
          {programs.map((program) => (
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
      )}
    </div>
  );
}
