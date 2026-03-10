interface Props {
  content: string;
  isOpen: boolean;
}

export default function FrameTooltip({ content, isOpen }: Props) {
  if (!isOpen) return null;

  return (
    <div className="mt-3 w-56 rounded-lg border border-zinc-700/50 bg-zinc-800/90 backdrop-blur-sm px-3 py-2 text-[11px] text-zinc-400 leading-relaxed shadow-lg shadow-zinc-950/50">
      {content}
    </div>
  );
}
