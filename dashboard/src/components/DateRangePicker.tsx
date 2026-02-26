export type DateRange = "7d" | "30d" | "All";

interface Props {
  readonly selected: DateRange;
  readonly onSelect: (range: DateRange) => void;
}

const RANGES: readonly DateRange[] = ["7d", "30d", "All"];

export function DateRangePicker({ selected, onSelect }: Props) {
  return (
    <div className="inline-flex rounded-md border border-tokamak-border">
      {RANGES.map((range) => (
        <button
          key={range}
          onClick={() => onSelect(range)}
          className={`px-3 py-1.5 text-sm ${
            selected === range
              ? "bg-tokamak-accent text-white"
              : "bg-tokamak-card text-slate-400 hover:text-white"
          } ${range === "7d" ? "rounded-l-md" : ""} ${range === "All" ? "rounded-r-md" : ""}`}
        >
          {range}
        </button>
      ))}
    </div>
  );
}
