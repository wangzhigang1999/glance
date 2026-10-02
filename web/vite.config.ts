import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") } },
  build: {
    outDir: "dist",
    assetsInlineLimit: 0,
    rollupOptions: {
      output: {
        entryFileNames: "app.js",
        chunkFileNames: "[name].js",
        assetFileNames: "app.[ext]",
      },
    },
  },
  server: {
    port: 5173,
    proxy: Object.fromEntries(
      ["/api", "/screen.bmp", "/next", "/logs.json"].map((p) => [
        p,
        {
          target: process.env.DEVICE_URL || "http://192.168.1.20",
          changeOrigin: true,
          configure(proxy: any) {
            proxy.on("proxyReq", (req: any) => {
              // Development only: preserve the firmware's same-origin boundary on its host.
              req.setHeader(
                "Origin",
                process.env.DEVICE_URL || "http://192.168.1.20",
              );
              req.removeHeader("Sec-Fetch-Site");
            });
          },
        },
      ]),
    ),
  },
});
