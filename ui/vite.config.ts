/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// Tauri dev 慣例 port 1420；純 web 開發（mock adapter）亦可任意 port。
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 1420, strictPort: false },
  test: {
    environment: 'node',
    include: ['src/**/*.test.{ts,tsx}'],
    // CSS 結構測試以 `index.css?raw` 讀真正的樣式表；vitest 預設把 CSS 換成空字串，`?raw` 也會變空。
    css: { include: [/\.css\?raw$/] },
  },
})
