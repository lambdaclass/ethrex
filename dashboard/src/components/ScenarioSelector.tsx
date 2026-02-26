interface Props {
  readonly scenarios: readonly string[];
  readonly selected: string;
  readonly onSelect: (scenario: string) => void;
}

export function ScenarioSelector({ scenarios, selected, onSelect }: Props) {
  return (
    <select
      value={selected}
      onChange={(e) => onSelect(e.target.value)}
      className="rounded-md border border-tokamak-border bg-tokamak-card px-3 py-1.5 text-sm text-white"
    >
      {scenarios.map((s) => (
        <option key={s} value={s}>{s}</option>
      ))}
    </select>
  );
}
