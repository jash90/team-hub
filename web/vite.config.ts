import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

const api = process.env.TEAM_HUB_API ?? 'http://127.0.0.1:8080';
const root = fileURLToPath(new URL('.', import.meta.url));

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { '@': `${root}src` },
    // Module screens live outside this folder; they must use this folder's copy of React.
    dedupe: ['react', 'react-dom', 'react-markdown', 'remark-gfm'],
  },
  server: {
    fs: { allow: ['..'] },
    proxy: {
      '/api': { target: api, changeOrigin: false },
      '/hooks': { target: api, changeOrigin: false },
    },
  },
  build: { outDir: 'dist', emptyOutDir: true, chunkSizeWarningLimit: 1500 },
});
