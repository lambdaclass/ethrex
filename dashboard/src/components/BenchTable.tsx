import type { BenchResult } from "@/types";
import { formatNs } from "@/lib/format";

interface Props {
  readonly results: readonly BenchResult[];
}

export function BenchTable({ results }: Props) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm text-slate-300">
        <thead className="border-b border-tokamak-border text-xs uppercase text-slate-500">
          <tr>
            <th className="px-4 py-3">Scenario</th>
            <th className="px-4 py-3">Mean</th>
            <th className="px-4 py-3">Std Dev</th>
            <th className="px-4 py-3">95% CI</th>
            <th className="px-4 py-3">Runs</th>
          </tr>
        </thead>
        <tbody>
          {results.map((r) => {
            const meanNs = r.stats?.mean_ns ?? r.total_duration_ns / r.runs;
            return (
              <tr key={r.scenario} className="border-b border-tokamak-border/50">
                <td className="px-4 py-3 font-medium text-white">{r.scenario}</td>
                <td className="px-4 py-3">{formatNs(meanNs)}</td>
                <td className="px-4 py-3">{r.stats ? formatNs(r.stats.stddev_ns) : "\u2014"}</td>
                <td className="px-4 py-3">
                  {r.stats
                    ? `${formatNs(r.stats.ci_lower_ns)} \u2013 ${formatNs(r.stats.ci_upper_ns)}`
                    : "\u2014"}
                </td>
                <td className="px-4 py-3">{r.runs}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
