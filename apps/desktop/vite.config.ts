import { defineConfig } from 'vite';

// Tauri 会在开发时读取固定端口，因此关掉端口漂移
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
  },
});
