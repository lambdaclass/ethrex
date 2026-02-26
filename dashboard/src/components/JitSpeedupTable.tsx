import type { JitBenchResult } from "@/types";
import { formatNs, formatSpeedup } from "@/lib/format";

interface Props {
  readonly results: readonly JitBenchResult[];
}

function speedupColor(speedup: number | null): string {
  if (speedup === null) return "text-slate-500";
  if (speedup >= 2) return "text-tokamak-green";
  if (speedup >= 1.5) return "text-tokamak-yellow";
  return "text-slate-400";
}

export function JitSpeedupTable({ results }: Props) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm text-slate-300">
        <thead className="border-b border-tokamak-border text-xs uppercase text-slate-500">
          <tr>
            <th className="px-4 py-3">Scenario</th>
            <th className="px-4 py-3">Interpreter</th>
            <th className="px-4 py-3">JIT</th>
            <th className="px-4 py-3">Speedup</th>
            <th className="px-4 py-3">Status</th>
          </tr>
        </thead>
        <tbody>
          {results.map((r) => (
            <tr key={r.scenario} className="border-b border-tokamak-border/50">
              <td className="px-4 py-3 font-medium text-white">{r.scenario}</td>
              <td className="px-4 py-3">{formatNs(r.interpreter_ns / r.runs)}</td>
              <td className="px-4 py-3">
                {r.jit_ns !== null ? formatNs(r.jit_ns / r.runs) : "\u2014"}
              </td>
              <td className={`px-4 py-3 font-medium ${speedupColor(r.speedup)}`}>
                {formatSpeedup(r.speedup)}
              </td>
              <td className="px-4 py-3 text-xs">
                {r.speedup !== null ? (
                  <span className="text-tokamak-green">JIT compiled</span>
                ) : (
                  <span className="text-slate-500">Interpreter only</span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
