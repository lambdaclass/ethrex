import type { CrossClientScenario } from "@/types";
import { formatNs } from "@/lib/format";
import { COLORS } from "@/lib/constants";

interface Props {
  readonly scenarios: readonly CrossClientScenario[];
}

function findClient(scenario: CrossClientScenario, name: string): number | null {
  const entry = scenario.results.find((r) => r.client_name === name);
  return entry ? entry.mean_ns : null;
}

function ratioLabel(clientNs: number | null, ethrexNs: number): string {
  if (clientNs === null) return "\u2014";
  const ratio = clientNs / ethrexNs;
  return `${ratio.toFixed(2)}x`;
}

function ratioColor(clientNs: number | null, ethrexNs: number): string {
  if (clientNs === null) return "text-slate-500";
  const ratio = clientNs / ethrexNs;
  if (ratio > 1.2) return "text-tokamak-green";
  if (ratio > 1.0) return "text-slate-300";
  return "text-tokamak-red";
}

export function CrossClientTable({ scenarios }: Props) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm text-slate-300">
        <thead className="border-b border-tokamak-border text-xs uppercase text-slate-500">
          <tr>
            <th className="px-4 py-3">Scenario</th>
            <th className="px-4 py-3" style={{ color: COLORS.ethrex }}>ethrex</th>
            <th className="px-4 py-3" style={{ color: COLORS.geth }}>Geth</th>
            <th className="px-4 py-3" style={{ color: COLORS.reth }}>Reth</th>
            <th className="px-4 py-3">vs Geth</th>
            <th className="px-4 py-3">vs Reth</th>
          </tr>
        </thead>
        <tbody>
          {scenarios.map((sc) => {
            const gethNs = findClient(sc, "geth");
            const rethNs = findClient(sc, "reth");
            return (
              <tr key={sc.scenario} className="border-b border-tokamak-border/50">
                <td className="px-4 py-3 font-medium text-white">{sc.scenario}</td>
                <td className="px-4 py-3">{formatNs(sc.ethrex_mean_ns)}</td>
                <td className="px-4 py-3">{gethNs !== null ? formatNs(gethNs) : "\u2014"}</td>
                <td className="px-4 py-3">{rethNs !== null ? formatNs(rethNs) : "\u2014"}</td>
                <td className={`px-4 py-3 font-medium ${ratioColor(gethNs, sc.ethrex_mean_ns)}`}>
                  {ratioLabel(gethNs, sc.ethrex_mean_ns)}
                </td>
                <td className={`px-4 py-3 font-medium ${ratioColor(rethNs, sc.ethrex_mean_ns)}`}>
                  {ratioLabel(rethNs, sc.ethrex_mean_ns)}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
