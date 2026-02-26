interface Props {
  readonly currentPath?: string;
}

const NAV_ITEMS = [
  { label: "Dashboard", href: "/" },
  { label: "Trends", href: "/trends" },
  { label: "Compare", href: "/compare" },
] as const;

export function Header({ currentPath = "/" }: Props) {
  return (
    <header className="border-b border-tokamak-border bg-tokamak-bg">
      <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-3">
        <a href="/" className="text-lg font-bold text-white">
          Tokamak <span className="text-tokamak-accent">Bench</span>
        </a>
        <nav className="flex gap-4">
          {NAV_ITEMS.map(({ label, href }) => (
            <a
              key={href}
              href={href}
              className={`text-sm ${
                currentPath === href ? "text-white" : "text-slate-400 hover:text-white"
              }`}
            >
              {label}
            </a>
          ))}
        </nav>
      </div>
    </header>
  );
}
