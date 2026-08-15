import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'path';

const apiTarget = process.env.API_PROXY_TARGET || 'http://127.0.0.1:9080';

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { '@': path.resolve(__dirname, 'src') },
  },
  server: {
    host: '127.0.0.1',
    port: 5173,
    allowedHosts: true,
    proxy: {
      '/api': { target: apiTarget, changeOrigin: true },
      '/live': { target: apiTarget },
      '/ready': { target: apiTarget },
      '/healthz': { target: apiTarget },
      '/readyz': { target: apiTarget },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
});
