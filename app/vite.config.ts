import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react-swc";
import path from "path";
import { VitePWA } from "vite-plugin-pwa";
import { componentTagger } from "lovable-tagger";

// https://vitejs.dev/config/
export default defineConfig(({ mode }) => {
  const plugins: (Plugin | Plugin[])[] = [react()];
  const enablePwa = process.env.VITE_PREVIA_ENABLE_PWA === "true";

  if (enablePwa) {
    plugins.push(
      VitePWA({
        filename: "precache-sw.js",
        injectRegister: "auto",
        registerType: "autoUpdate",
        manifest: false,
        workbox: {
          clientsClaim: true,
          cleanupOutdatedCaches: true,
          navigateFallbackDenylist: [/^\/~oauth/],
          skipWaiting: true,
        },
        devOptions: {
          enabled: true,
        },
      }),
    );
  }
  
  if (mode === "development") {
    plugins.push(componentTagger());
  }
  
  return {
    server: {
      host: "::",
      port: 8080,
      hmr: {
        overlay: false,
      },
    },
    plugins: plugins as Plugin[],
    resolve: {
      alias: {
        "@": path.resolve(__dirname, "./src"),
      },
      dedupe: ["react", "react-dom", "react/jsx-runtime"],
    },
    build: {
      rollupOptions: {
        output: {
          manualChunks(id) {
            if (!id.includes("node_modules")) return undefined;

            if (id.includes("@monaco-editor/react") || id.includes("monaco-editor")) {
              return "monaco";
            }

            if (id.includes("recharts") || id.includes("d3-")) {
              return "charts";
            }

            if (id.includes("react-markdown") || id.includes("remark-") || id.includes("micromark") || id.includes("mdast-") || id.includes("hast-")) {
              return "markdown";
            }

            if (id.includes("react-router") || id.includes("@remix-run")) {
              return "router";
            }

            return undefined;
          },
        },
      },
    },
  };
});
