import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { resolve } from 'node:path'

// 多頁：每個網址一個 index.html，靜態主機直接對應資料夾。
export default defineConfig({
  plugins: [react()],
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
        verify: resolve(__dirname, 'auth/verify/index.html'),
        welcome: resolve(__dirname, 'welcome/index.html'),
        privacy: resolve(__dirname, 'privacy/index.html'),
        terms: resolve(__dirname, 'terms/index.html'),
        support: resolve(__dirname, 'support/index.html'),
      },
    },
  },
  test: { environment: 'jsdom' },
})
