import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./src/**/*.{astro,html,tsx,ts}"],
  theme: {
    extend: {
      colors: {
        tokamak: {
          bg: "#0f1117",
          card: "#1a1d2e",
          border: "#2a2d3e",
          accent: "#6366f1",
          green: "#22c55e",
          yellow: "#eab308",
          red: "#ef4444",
        },
      },
    },
  },
  plugins: [],
};

export default config;
