import type { AuthMethod } from '../lib/api';

interface Props {
  value: AuthMethod;
  onChange: (method: AuthMethod) => void;
}

export default function AuthMethodToggle({ value, onChange }: Props) {
  return (
    <div className="flex items-center gap-1 rounded-lg border border-zinc-700 bg-zinc-800/50 p-0.5">
      <button
        type="button"
        onClick={() => onChange('passkey')}
        className={`rounded-md px-3 py-1.5 text-xs font-medium transition-colors cursor-pointer ${
          value === 'passkey'
            ? 'bg-indigo-600 text-white'
            : 'text-zinc-400 hover:text-zinc-200'
        }`}
      >
        Passkey
      </button>
      <button
        type="button"
        onClick={() => onChange('ephemeral')}
        className={`rounded-md px-3 py-1.5 text-xs font-medium transition-colors cursor-pointer ${
          value === 'ephemeral'
            ? 'bg-indigo-600 text-white'
            : 'text-zinc-400 hover:text-zinc-200'
        }`}
      >
        Ephemeral Key
      </button>
    </div>
  );
}
