import { useState } from 'react';
import type { TxResult as TxResultType } from '../lib/api';

const BLOCKSCOUT_URL: string | undefined = import.meta.env.VITE_BLOCKSCOUT_URL;

function getBlockscoutTxUrl(txHash: string): string {
  if (BLOCKSCOUT_URL) return `${BLOCKSCOUT_URL}/tx/${txHash}`;
  const { protocol, hostname } = window.location;
  return `${protocol}//${hostname}:8082/tx/${txHash}`;
}

export default function TxResult({ result }: { result: TxResultType }) {
  const [copied, setCopied] = useState(false);

  const copyHash = async () => {
    if (result.txHash) {
      await navigator.clipboard.writeText(result.txHash);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  const txUrl = result.txHash ? getBlockscoutTxUrl(result.txHash) : '';

  return (
    <div
      className={`mt-4 rounded-lg border p-4 ${
        result.success
          ? 'border-emerald-500/30 bg-emerald-950/30'
          : 'border-red-500/30 bg-red-950/30'
      }`}
    >
      <div className="flex items-center gap-2 mb-2">
        <span
          className={`inline-block h-2.5 w-2.5 rounded-full ${
            result.success ? 'bg-emerald-400' : 'bg-red-400'
          }`}
        />
        <span className="font-medium text-sm text-zinc-200">
          {result.success ? 'Transaction Successful' : 'Transaction Failed'}
        </span>
      </div>

      {result.txHash && (
        <div className="flex items-center gap-2 mt-2">
          <span className="text-xs text-zinc-500">Hash:</span>
          {txUrl ? (
            <a
              href={txUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="text-xs text-blue-400 hover:text-blue-300 font-mono truncate max-w-[360px] underline underline-offset-2"
            >
              {result.txHash}
            </a>
          ) : (
            <code className="text-xs text-zinc-300 font-mono truncate max-w-[360px]">
              {result.txHash}
            </code>
          )}
          <button
            onClick={copyHash}
            className="text-xs text-zinc-500 hover:text-zinc-300 transition-colors cursor-pointer"
          >
            {copied ? 'Copied' : 'Copy'}
          </button>
        </div>
      )}

      {result.gasUsed && (
        <div className="mt-1">
          <span className="text-xs text-zinc-500">Gas used: </span>
          <span className="text-xs text-zinc-400 font-mono">{result.gasUsed}</span>
        </div>
      )}

      {result.deployedAddress && (
        <div className="mt-1">
          <span className="text-xs text-zinc-500">Deployed at: </span>
          <code className="text-xs text-violet-400 font-mono">{result.deployedAddress}</code>
        </div>
      )}

      {result.frameReceipts && result.frameReceipts.length > 0 && (
        <div className="mt-3 space-y-1">
          <span className="text-xs text-zinc-500 font-medium">Frame Results:</span>
          {result.frameReceipts.map((fr, i) => (
            <div key={i} className="flex items-center gap-2 ml-2">
              <span
                className={`inline-block h-1.5 w-1.5 rounded-full ${
                  fr.status ? 'bg-emerald-400' : 'bg-red-400'
                }`}
              />
              <span className="text-xs text-zinc-400">
                {fr.mode}
              </span>
              <span className={`text-xs ${fr.status ? 'text-emerald-400' : 'text-red-400'}`}>
                {fr.status ? 'OK' : 'REVERTED'}
              </span>
              <span className="text-xs text-zinc-600 font-mono">
                (gas: {fr.gasUsed})
              </span>
            </div>
          ))}
        </div>
      )}

      {result.error && (
        <div className="mt-2">
          <span className="text-xs text-red-400">{result.error}</span>
        </div>
      )}
    </div>
  );
}
