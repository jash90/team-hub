import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

const root = fileURLToPath(new URL('.', import.meta.url));
const repository = fileURLToPath(new URL('../../..', import.meta.url));

// The look, the building blocks and the generated types come from the web interface in this
// repository, so the two cannot drift apart: `@web` is `web/src`, `@modules` is `modules`.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  resolve: {
    alias: { '@web': `${repository}web/src`, '@modules': `${repository}modules`, '@': `${root}src` },
    // Files outside this folder must use this folder's copy of React.
    dedupe: ['react', 'react-dom', 'react-markdown', 'remark-gfm'],
  },
  server: { port: 1430, strictPort: true, fs: { allow: [repository] } },
  build: { outDir: 'dist', emptyOutDir: true, target: 'es2022', chunkSizeWarningLimit: 1500 },
});
