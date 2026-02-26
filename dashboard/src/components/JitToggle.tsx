interface Props {
  readonly enabled: boolean;
  readonly onToggle: (enabled: boolean) => void;
}

export function JitToggle({ enabled, onToggle }: Props) {
  return (
    <button
      onClick={() => onToggle(!enabled)}
      className={`inline-flex items-center gap-2 rounded-md border px-3 py-1.5 text-sm ${
        enabled
          ? "border-tokamak-green bg-tokamak-green/10 text-tokamak-green"
          : "border-tokamak-border bg-tokamak-card text-slate-400"
      }`}
    >
      <span className={`inline-block h-2 w-2 rounded-full ${enabled ? "bg-tokamak-green" : "bg-slate-600"}`} />
      JIT
    </button>
  );
}
