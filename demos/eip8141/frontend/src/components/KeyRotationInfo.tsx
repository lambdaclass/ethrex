import type { TxResult } from '../lib/api';

interface Props {
  result: TxResult;
}

export default function KeyRotationInfo({ result }: Props) {
  if (!result.oldSigner || !result.newSigner) return null;

  return (
    <div className="rounded-t-lg border border-b-0 border-zinc-700 bg-zinc-900/50 px-3 py-2">
      <span className="text-xs text-zinc-500">Key rotation: </span>
      <span className="text-xs text-zinc-400 font-mono">{result.oldSigner.slice(0, 10)}...</span>
      <span className="text-xs text-zinc-500 mx-1">{'->'}</span>
      <span className="text-xs text-emerald-400 font-mono">{result.newSigner.slice(0, 10)}...</span>
    </div>
  );
}
