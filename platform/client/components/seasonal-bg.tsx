"use client";

import { useMemo } from "react";

type Season = "spring" | "summer" | "autumn" | "winter";

function getSeason(): Season {
  const month = new Date().getMonth() + 1; // 1-12
  if (month >= 3 && month <= 5) return "spring";
  if (month >= 6 && month <= 8) return "summer";
  if (month >= 9 && month <= 11) return "autumn";
  return "winter";
}

const SEASON_CONFIG: Record<Season, { particles: string[]; colors: string[]; speed: string; label: string }> = {
  spring: {
    particles: ["\u{1F338}", "\u{1F33C}", "\u{1F33A}"], // cherry blossom, sunflower-like, hibiscus
    colors: ["#fbb6ce", "#f9a8d4", "#fbcfe8"],
    speed: "fall-slow",
    label: "Spring",
  },
  summer: {
    particles: ["\u{2728}", "\u{2600}\u{FE0F}", "\u{1F31F}"],
    colors: ["#fde68a", "#fed7aa", "#fef3c7"],
    speed: "float",
    label: "Summer",
  },
  autumn: {
    particles: ["\u{1F341}", "\u{1F342}", "\u{1F343}"],
    colors: ["#fdba74", "#f97316", "#dc2626"],
    speed: "fall-drift",
    label: "Autumn",
  },
  winter: {
    particles: ["\u{2744}\u{FE0F}", "\u{2B50}", "\u{2728}"],
    colors: ["#e0e7ff", "#c7d2fe", "#dbeafe"],
    speed: "fall-slow",
    label: "Winter",
  },
};

function seededRandom(seed: number): number {
  const x = Math.sin(seed * 9301 + 49297) * 49297;
  return x - Math.floor(x);
}

export function SeasonalBackground() {
  const season = getSeason();
  const config = SEASON_CONFIG[season];

  const particles = useMemo(() => {
    return Array.from({ length: 15 }, (_, i) => ({
      id: i,
      char: config.particles[i % config.particles.length],
      left: seededRandom(i * 7 + 1) * 100,
      delay: seededRandom(i * 13 + 2) * 8,
      duration: 8 + seededRandom(i * 17 + 3) * 7,
      size: 12 + seededRandom(i * 23 + 4) * 14,
      opacity: 0.15 + seededRandom(i * 29 + 5) * 0.25,
      drift: -30 + seededRandom(i * 31 + 6) * 60,
    }));
  }, [config.particles]);

  return (
    <>
      <div className="fixed inset-0 pointer-events-none overflow-hidden z-0" aria-hidden="true">
        {particles.map((p) => (
          <span
            key={p.id}
            className={`absolute animate-${config.speed}`}
            style={{
              left: `${p.left}%`,
              top: "-5%",
              fontSize: `${p.size}px`,
              opacity: p.opacity,
              animationDelay: `${p.delay}s`,
              animationDuration: `${p.duration}s`,
              "--drift": `${p.drift}px`,
            } as React.CSSProperties}
          >
            {p.char}
          </span>
        ))}
      </div>

      <style jsx global>{`
        @keyframes fall-slow {
          0% {
            transform: translateY(-5vh) translateX(0) rotate(0deg);
            opacity: 0;
          }
          10% {
            opacity: var(--particle-opacity, 0.3);
          }
          90% {
            opacity: var(--particle-opacity, 0.3);
          }
          100% {
            transform: translateY(105vh) translateX(var(--drift, 20px)) rotate(360deg);
            opacity: 0;
          }
        }

        @keyframes fall-drift {
          0% {
            transform: translateY(-5vh) translateX(0) rotate(0deg);
            opacity: 0;
          }
          10% {
            opacity: var(--particle-opacity, 0.3);
          }
          25% {
            transform: translateY(25vh) translateX(calc(var(--drift, 20px) * 0.5)) rotate(90deg);
          }
          50% {
            transform: translateY(50vh) translateX(var(--drift, 20px)) rotate(180deg);
          }
          75% {
            transform: translateY(75vh) translateX(calc(var(--drift, 20px) * 0.3)) rotate(270deg);
          }
          90% {
            opacity: var(--particle-opacity, 0.3);
          }
          100% {
            transform: translateY(105vh) translateX(calc(var(--drift, 20px) * 0.8)) rotate(360deg);
            opacity: 0;
          }
        }

        @keyframes float {
          0% {
            transform: translateY(105vh) translateX(0) scale(0.8);
            opacity: 0;
          }
          10% {
            opacity: var(--particle-opacity, 0.3);
          }
          50% {
            transform: translateY(50vh) translateX(var(--drift, 20px)) scale(1);
          }
          90% {
            opacity: var(--particle-opacity, 0.3);
          }
          100% {
            transform: translateY(-5vh) translateX(calc(var(--drift, 20px) * -1)) scale(0.8);
            opacity: 0;
          }
        }

        .animate-fall-slow {
          animation: fall-slow linear infinite;
        }
        .animate-fall-drift {
          animation: fall-drift linear infinite;
        }
        .animate-float {
          animation: float linear infinite;
        }
      `}</style>
    </>
  );
}
