import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

const api = process.env.TEAM_HUB_API ?? 'http://127.0.0.1:8080';
const root = fileURLToPath(new URL('.', import.meta.url));

/** The id of the module whose `web/index.tsx` this is, if it is one. */
function moduleOf(path: string | null | undefined): string | undefined {
  return /(?:^|[\\/])modules[\\/]([^\\/]+)[\\/]web[\\/]index\.tsx$/.exec(path ?? '')?.[1];
}

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
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rolldownOptions: {
      output: {
        // A module's screens are a chunk of their own (see src/modules.ts), and so are its
        // styles; both are named after the module.
        chunkFileNames: (chunk) => {
          const module = moduleOf(chunk.facadeModuleId);

          return module ? `assets/module-${module}-[hash].js` : 'assets/[name]-[hash].js';
        },
        assetFileNames: (asset) => {
          const module = moduleOf(asset.originalFileNames[0]);

          return module ? `assets/module-${module}-[hash][extname]` : 'assets/[name]-[hash][extname]';
        },
      },
    },
  },
});
