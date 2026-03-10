export default function Footer() {
  return (
    <footer className="border-t border-zinc-800/50 mt-16 py-6">
      <div className="max-w-6xl mx-auto px-6 flex items-center justify-center gap-4 text-xs text-zinc-600">
        <a
          href="https://github.com/lambdaclass/ethrex"
          target="_blank"
          rel="noopener noreferrer"
          className="hover:text-zinc-400 transition-colors"
        >
          ethrex
        </a>
        <span>·</span>
        <a
          href="https://eips.ethereum.org/EIPS/eip-8141"
          target="_blank"
          rel="noopener noreferrer"
          className="hover:text-zinc-400 transition-colors"
        >
          EIP-8141
        </a>
      </div>
    </footer>
  );
}
