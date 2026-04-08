import type { Config } from "tailwindcss";

const config: Config = {
  content: [
    "./src/pages/**/*.{js,ts,jsx,tsx,mdx}",
    "./src/components/**/*.{js,ts,jsx,tsx,mdx}",
    "./src/app/**/*.{js,ts,jsx,tsx,mdx}"
  ],
  theme: {
    extend: {
      colors: {
        surface: "#f5f5f7",
        panel: "#ffffff",
        muted: "#f2f2f7",
        primary: "#0a84ff"
      },
      boxShadow: {
        soft: "0 10px 30px rgba(17, 24, 39, 0.06)",
        panel: "0 18px 48px rgba(15, 23, 42, 0.08)"
      },
      fontFamily: {
        sans: [
          "SF Pro Display",
          "SF Pro Text",
          "ui-sans-serif",
          "system-ui",
          "-apple-system",
          "BlinkMacSystemFont",
          "sans-serif"
        ]
      }
    }
  },
  plugins: []
};

export default config;
