import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  envPrefix: ['VITE_'],
  server: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: 'es2022',
    sourcemap: process.env.TAURI_ENV_DEBUG === 'true',
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.{ts,tsx}'],
    clearMocks: true,
    restoreMocks: true,
  },
});
