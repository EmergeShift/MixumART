import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Tauri 约定：固定 1420，与 src-tauri/tauri.conf.json 的 devUrl 对齐。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
})
