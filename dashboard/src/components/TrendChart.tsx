import {
  Line, XAxis, YAxis, Tooltip,
  Area, CartesianGrid, ResponsiveContainer, ComposedChart,
} from "recharts";
import type { Payload } from "recharts/types/component/DefaultTooltipContent";
import type { TrendPoint } from "@/lib/data";
import { COLORS } from "@/lib/constants";
import { formatNs, formatCommit } from "@/lib/format";

interface Props {
  readonly data: readonly TrendPoint[];
  readonly showCi?: boolean;
}

export function TrendChart({ data, showCi = true }: Props) {
  if (data.length === 0) {
    return <p className="text-center text-slate-500">No trend data available</p>;
  }

  return (
    <ResponsiveContainer width="100%" height={320}>
      <ComposedChart data={[...data]} margin={{ top: 8, right: 16, bottom: 8, left: 16 }}>
        <CartesianGrid strokeDasharray="3 3" stroke={COLORS.grid} />
        <XAxis
          dataKey="date"
          stroke={COLORS.text}
          tick={{ fontSize: 12 }}
        />
        <YAxis
          stroke={COLORS.text}
          tick={{ fontSize: 12 }}
          tickFormatter={(v: number) => formatNs(v)}
        />
        <Tooltip
          contentStyle={{ backgroundColor: "#1a1d2e", border: "1px solid #2a2d3e" }}
          labelStyle={{ color: "#94a3b8" }}
          formatter={(value: number) => [formatNs(value), "Mean"] as const}
          labelFormatter={(label: string, payload: Payload<number, "Mean">[]) => {
            const point = payload[0]?.payload as TrendPoint | undefined;
            return point ? `${label} (${formatCommit(point.commit)})` : label;
          }}
        />
        {showCi && (
          <Area
            dataKey="ci_upper_ns"
            stroke="none"
            fill={COLORS.ci_band}
            type="monotone"
          />
        )}
        {showCi && (
          <Area
            dataKey="ci_lower_ns"
            stroke="none"
            fill="#0f1117"
            type="monotone"
          />
        )}
        <Line
          dataKey="mean_ns"
          stroke={COLORS.interpreter}
          strokeWidth={2}
          dot={{ r: 3 }}
          type="monotone"
        />
      </ComposedChart>
    </ResponsiveContainer>
  );
}
