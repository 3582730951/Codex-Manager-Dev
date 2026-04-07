import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: "standalone",
  transpilePackages: ["@codex-manager/contracts"]
};

export default nextConfig;

