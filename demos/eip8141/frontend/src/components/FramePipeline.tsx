import { useState } from 'react';
import FrameTooltip from './FrameTooltip';

export interface FrameConfig {
  mode: 'VERIFY' | 'SENDER' | 'DEFAULT';
  label: string;
  target: string;
  tooltip: string;
}

export type ExecutionPhase = 'idle' | 'executing' | 'done' | 'error';

export interface ExecutionState {
  phase: ExecutionPhase;
  activeFrameIndex?: number;
  errorFrameIndex?: number;
}

function getFrameStateClasses(
  index: number,
  state: ExecutionState,
): string {
  const { phase, activeFrameIndex, errorFrameIndex } = state;

  switch (phase) {
    case 'idle':
      return 'opacity-50 border-zinc-700';
    case 'executing':
      if (activeFrameIndex !== undefined) {
        if (index < activeFrameIndex) return 'opacity-100 border-emerald-500';
        if (index === activeFrameIndex)
          return 'opacity-100 border-indigo-500 shadow-lg shadow-indigo-500/20';
        return 'opacity-50 border-zinc-700';
      }
      return 'opacity-50 border-zinc-700';
    case 'done':
      return 'opacity-100 border-emerald-500';
    case 'error':
      if (errorFrameIndex !== undefined) {
        if (index < errorFrameIndex) return 'opacity-100 border-emerald-500';
        if (index === errorFrameIndex) return 'opacity-100 border-red-500';
        return 'opacity-30 border-zinc-700';
      }
      return 'opacity-100 border-red-500';
    default:
      return 'opacity-50 border-zinc-700';
  }
}

function getModeColor(mode: FrameConfig['mode']): string {
  switch (mode) {
    case 'VERIFY':
      return 'text-amber-400';
    case 'SENDER':
      return 'text-indigo-400';
    case 'DEFAULT':
      return 'text-violet-400';
  }
}

interface Props {
  frames: FrameConfig[];
  executionState: ExecutionState;
}

export default function FramePipeline({ frames, executionState }: Props) {
  const [openTooltip, setOpenTooltip] = useState<number | null>(null);

  const toggleTooltip = (index: number) => {
    setOpenTooltip(prev => (prev === index ? null : index));
  };

  return (
    <div className="flex flex-col gap-2">
      <span className="text-xs text-zinc-500 font-medium uppercase tracking-wider mb-1">
        Frame Pipeline
      </span>

      <div className="flex items-start gap-1 flex-wrap">
        {frames.map((frame, i) => (
          <div key={i} className="flex items-start gap-1">
            {/* Frame box */}
            <div className="flex flex-col items-center">
              <div
                className={`relative rounded-lg border bg-zinc-900/50 px-3 py-2.5 min-w-[110px] text-center transition-all duration-500 ${getFrameStateClasses(i, executionState)}`}
              >
                {/* Active pulse indicator */}
                {executionState.phase === 'executing' &&
                  executionState.activeFrameIndex === i && (
                    <span className="absolute top-1.5 right-1.5 inline-block h-2 w-2 rounded-full bg-indigo-400 animate-pulse" />
                  )}

                <div className={`text-xs font-semibold ${getModeColor(frame.mode)}`}>
                  {frame.mode}
                </div>
                <div className="text-[10px] text-zinc-400 mt-0.5 leading-tight">
                  {frame.label}
                </div>
                <div className="text-[10px] text-zinc-600 mt-0.5">
                  {frame.target}
                </div>

                {/* Info icon */}
                <button
                  onClick={() => toggleTooltip(i)}
                  className={`absolute -bottom-1.5 left-1/2 -translate-x-1/2 w-4 h-4 rounded-full text-[9px] leading-none flex items-center justify-center transition-colors cursor-pointer ${
                    openTooltip === i
                      ? 'bg-indigo-600 text-white'
                      : 'bg-zinc-800 text-zinc-500 hover:text-zinc-300 hover:bg-zinc-700'
                  }`}
                >
                  i
                </button>
              </div>

              {/* Tooltip */}
              <FrameTooltip
                content={frame.tooltip}
                isOpen={openTooltip === i}
              />
            </div>

            {/* Arrow connector */}
            {i < frames.length - 1 && (
              <div className="flex items-center self-center mt-1">
                <div className="w-4 h-px bg-zinc-600" />
                <div className="w-0 h-0 border-t-[3px] border-t-transparent border-b-[3px] border-b-transparent border-l-[5px] border-l-zinc-600" />
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
