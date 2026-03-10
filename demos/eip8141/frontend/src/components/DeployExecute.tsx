import { useState } from 'react';
import type { StoredCredential } from '../lib/passkey';
import { signChallenge } from '../lib/passkey';
import * as api from '../lib/api';
import type { TxResult as TxResultType } from '../lib/api';
import TxResult from './TxResult';
import FramePipeline from './FramePipeline';
import type { FrameConfig, ExecutionState } from './FramePipeline';

const FRAMES: FrameConfig[] = [
  {
    mode: 'VERIFY',
    label: 'scope = 2',
    target: 'account',
    tooltip:
      'Calls verifyAndPay() on the account contract. Uses TXPARAMLOAD(0x08) to read the sig_hash, verifies the WebAuthn P256 signature, then calls APPROVE(scope=2) to authorize as both sender and gas payer.',
  },
  {
    mode: 'DEFAULT',
    label: 'deployer proxy',
    target: 'CREATE2',
    tooltip:
      'Calls the deployer proxy with salt + initCode. Uses CREATE2 to deploy the contract at a deterministic address.',
  },
  {
    mode: 'SENDER',
    label: 'execute()',
    target: 'account',
    tooltip:
      'Calls execute(target, value, data) on the account. Routes the post-deploy call through the account\'s execution logic.',
  },
];

interface Props {
  credential: StoredCredential;
}

export default function DeployExecute({ credential }: Props) {
  const [bytecode, setBytecode] = useState('');
  const [constructorArgs, setConstructorArgs] = useState('');
  const [callData, setCallData] = useState('');
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');
  const [history, setHistory] = useState<TxResultType[]>([]);

  const handleDeploy = async () => {
    if (!bytecode) {
      setError('Bytecode is required');
      return;
    }
    setLoading(true);
    setError('');
    try {
      setStatus('Building transaction...');
      const { sigHash } = await api.getSigHash('deploy-execute', {
        from: credential.address,
        bytecode,
        calldata: callData,
      });

      setStatus('Sign with your passkey...');
      const signed = await signChallenge(credential.id, sigHash);

      setStatus('Submitting transaction...');
      const txResult = await api.deployExecute({
        address: credential.address,
        bytecode,
        calldata: callData,
        signature: signed.signature,
        webauthn: signed.webauthn,
      });

      setHistory(prev => [txResult, ...prev]);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Deploy failed');
    } finally {
      setLoading(false);
      setStatus('');
    }
  };

  const executionState: ExecutionState = (() => {
    if (error && !loading) return { phase: 'error' as const, errorFrameIndex: 2 };
    if (!loading && history.length > 0 && history[0].success) return { phase: 'done' as const };
    if (status === 'Building transaction...') return { phase: 'executing' as const, activeFrameIndex: 0 };
    if (status === 'Sign with your passkey...') return { phase: 'executing' as const, activeFrameIndex: 0 };
    if (status === 'Submitting transaction...') return { phase: 'executing' as const, activeFrameIndex: 1 };
    return { phase: 'idle' as const };
  })();

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-8">
      <div>
        <h3 className="text-lg font-semibold text-zinc-100 mb-1">Deploy & Execute</h3>
        <p className="text-sm text-zinc-500 mb-5">
          Deploy a contract and call it in a single frame transaction.
        </p>

        <div className="space-y-4">
          <div>
            <label className="block text-xs text-zinc-400 mb-1.5">Contract Bytecode</label>
            <textarea
              placeholder="0x608060..."
              value={bytecode}
              onChange={e => setBytecode(e.target.value)}
              rows={3}
              className="w-full rounded-lg border border-zinc-700 bg-zinc-800/50 px-3 py-2 text-sm text-zinc-200 font-mono placeholder-zinc-600 focus:border-indigo-500 focus:outline-none resize-none"
            />
          </div>

          <div>
            <label className="block text-xs text-zinc-400 mb-1.5">
              Constructor Arguments (ABI-encoded, optional)
            </label>
            <input
              type="text"
              placeholder="0x..."
              value={constructorArgs}
              onChange={e => setConstructorArgs(e.target.value)}
              className="w-full rounded-lg border border-zinc-700 bg-zinc-800/50 px-3 py-2 text-sm text-zinc-200 font-mono placeholder-zinc-600 focus:border-indigo-500 focus:outline-none"
            />
          </div>

          <div>
            <label className="block text-xs text-zinc-400 mb-1.5">
              Post-Deploy Call Data (optional)
            </label>
            <input
              type="text"
              placeholder="0x..."
              value={callData}
              onChange={e => setCallData(e.target.value)}
              className="w-full rounded-lg border border-zinc-700 bg-zinc-800/50 px-3 py-2 text-sm text-zinc-200 font-mono placeholder-zinc-600 focus:border-indigo-500 focus:outline-none"
            />
          </div>

          <button
            onClick={handleDeploy}
            disabled={loading}
            className="w-full rounded-lg bg-violet-600 hover:bg-violet-500 disabled:opacity-50 disabled:cursor-not-allowed px-4 py-2.5 text-sm font-medium text-white transition-colors cursor-pointer"
          >
            {loading ? 'Processing...' : 'Deploy & Execute'}
          </button>
        </div>

        {status && (
          <div className="mt-3 flex items-center gap-2 rounded-lg border border-violet-500/30 bg-violet-950/30 px-3 py-2">
            <span className="inline-block h-2 w-2 rounded-full bg-violet-400 animate-pulse" />
            <span className="text-sm text-violet-300">{status}</span>
          </div>
        )}
        {error && <p className="text-sm text-red-400 mt-3">{error}</p>}
        {history.length > 0 && (
          <div className="mt-6 space-y-2">
            <h4 className="text-xs font-medium text-zinc-500 uppercase tracking-wider">Transaction History</h4>
            {history.map((r, i) => <TxResult key={i} result={r} />)}
          </div>
        )}
      </div>

      <div className="lg:pt-10">
        <FramePipeline frames={FRAMES} executionState={executionState} />
      </div>
    </div>
  );
}
