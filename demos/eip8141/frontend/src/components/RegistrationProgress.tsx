import { useState, useEffect } from 'react';
import { registerAccountStream } from '../lib/api';
import type { StoredCredential } from '../lib/passkey';

const BLOCKSCOUT_URL: string | undefined = import.meta.env.VITE_BLOCKSCOUT_URL;

function getBlockscoutTxUrl(txHash: string): string {
  if (BLOCKSCOUT_URL) return `${BLOCKSCOUT_URL}/tx/${txHash}`;
  const { hostname } = window.location;
  return `http://${hostname}:8082/tx/${txHash}`;
}

function getBlockscoutAddressUrl(address: string): string {
  if (BLOCKSCOUT_URL) return `${BLOCKSCOUT_URL}/address/${address}`;
  const { hostname } = window.location;
  return `http://${hostname}:8082/address/${address}`;
}

type StepId = 'passkey' | 'deploy' | 'fund' | 'mint';
type StepStatus = 'waiting' | 'pending' | 'done' | 'error';

interface Step {
  id: StepId;
  label: string;
  status: StepStatus;
  address?: string;
  txHash?: string;
  error?: string;
}

const INITIAL_STEPS: Step[] = [
  { id: 'passkey', label: 'Passkey created', status: 'done' },
  { id: 'deploy', label: 'Deploying smart account', status: 'waiting' },
  { id: 'fund', label: 'Funding with 10 ETH', status: 'waiting' },
  { id: 'mint', label: 'Minting 1,000,000 DEMO tokens', status: 'waiting' },
];

function StepIcon({ status }: { status: StepStatus }) {
  switch (status) {
    case 'done':
      return <span className="text-emerald-400 text-sm">&#10003;</span>;
    case 'pending':
      return <span className="inline-block h-3 w-3 rounded-full border-2 border-indigo-400 border-t-transparent animate-spin" />;
    case 'error':
      return <span className="text-red-400 text-sm">&#10007;</span>;
    default:
      return <span className="inline-block h-2 w-2 rounded-full bg-zinc-600" />;
  }
}

interface Props {
  credential: StoredCredential;
  onComplete: (address: string) => void;
  onError: (message: string) => void;
}

export default function RegistrationProgress({ credential, onComplete, onError }: Props) {
  const [steps, setSteps] = useState<Step[]>(INITIAL_STEPS);

  const completedCount = steps.filter(s => s.status === 'done').length;
  const progress = (completedCount / steps.length) * 100;

  useEffect(() => {
    let cancelled = false;

    async function run() {
      try {
        const response = await registerAccountStream(credential);
        const reader = response.body?.getReader();
        if (!reader) throw new Error('No response body');

        const decoder = new TextDecoder();
        let buffer = '';

        while (true) {
          const { done, value } = await reader.read();
          if (done || cancelled) break;

          buffer += decoder.decode(value, { stream: true });

          // Parse SSE events from buffer
          const parts = buffer.split('\n\n');
          buffer = parts.pop() ?? '';

          for (const part of parts) {
            const lines = part.split('\n');
            let event = '';
            let data = '';
            for (const line of lines) {
              if (line.startsWith('event: ')) event = line.slice(7);
              if (line.startsWith('data: ')) data = line.slice(6);
            }
            if (!event || !data) continue;

            if (event === 'step') {
              const parsed = JSON.parse(data) as {
                step: StepId;
                status: string;
                address?: string;
                txHash?: string;
              };
              setSteps(prev => prev.map(s =>
                s.id === parsed.step
                  ? {
                      ...s,
                      status: parsed.status === 'done' ? 'done' : 'pending',
                      label: parsed.status === 'done' ? doneLabel(parsed.step) : s.label,
                      address: parsed.address ?? s.address,
                      txHash: parsed.txHash ?? s.txHash,
                    }
                  : s
              ));
            } else if (event === 'complete') {
              const parsed = JSON.parse(data) as { address: string };
              if (!cancelled) onComplete(parsed.address);
            } else if (event === 'error') {
              const parsed = JSON.parse(data) as { step?: string; message: string };
              if (parsed.step) {
                setSteps(prev => prev.map(s =>
                  s.id === parsed.step
                    ? { ...s, status: 'error', error: parsed.message }
                    : s
                ));
              }
              if (!cancelled) onError(parsed.message);
            }
          }
        }
      } catch (err) {
        if (!cancelled) {
          onError(err instanceof Error ? err.message : 'Registration failed');
        }
      }
    }

    run();
    return () => { cancelled = true; };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="w-full max-w-md mx-auto">
      <h3 className="text-lg font-semibold text-zinc-100 mb-4">Creating your account</h3>

      <div className="space-y-3 mb-5">
        {steps.map(step => (
          <div key={step.id} className="flex items-start gap-3">
            <div className="mt-0.5 w-4 flex-shrink-0 flex justify-center">
              <StepIcon status={step.status} />
            </div>
            <div className="min-w-0">
              <span className={`text-sm ${step.status === 'done' ? 'text-zinc-200' : step.status === 'error' ? 'text-red-400' : step.status === 'pending' ? 'text-zinc-300' : 'text-zinc-500'}`}>
                {step.label}
              </span>

              {/* Artifact links */}
              {step.status === 'done' && (step.address || step.txHash) && (
                <div className="flex items-center gap-2 mt-0.5">
                  {step.address && (
                    <a
                      href={getBlockscoutAddressUrl(step.address)}
                      className="text-xs text-indigo-400 hover:text-indigo-300 font-mono transition-colors"
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      {step.address.slice(0, 8)}...{step.address.slice(-6)} &#8599;
                    </a>
                  )}
                  {step.txHash && (
                    <a
                      href={getBlockscoutTxUrl(step.txHash)}
                      className="text-xs text-zinc-500 hover:text-zinc-400 font-mono transition-colors"
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      tx: {step.txHash.slice(0, 10)}... &#8599;
                    </a>
                  )}
                </div>
              )}

              {/* Error message */}
              {step.status === 'error' && step.error && (
                <p className="text-xs text-red-400/80 mt-0.5">{step.error}</p>
              )}
            </div>
          </div>
        ))}
      </div>

      {/* Progress bar */}
      <div className="h-1.5 bg-zinc-800 rounded-full overflow-hidden">
        <div
          className="h-full bg-indigo-500 rounded-full transition-all duration-500 ease-out"
          style={{ width: `${progress}%` }}
        />
      </div>
      <p className="text-xs text-zinc-500 mt-1.5 text-right">{completedCount}/{steps.length}</p>
    </div>
  );
}

function doneLabel(step: StepId): string {
  switch (step) {
    case 'passkey': return 'Passkey created';
    case 'deploy': return 'Smart account deployed';
    case 'fund': return 'Funded with 10 ETH';
    case 'mint': return 'Minted 1,000,000 DEMO tokens';
  }
}
