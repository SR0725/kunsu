// 證據畫布沙盒 runtime 的獨立 bundle（PRD §24）：單一 IIFE、資產全部內嵌，輸出到 public/canvas-runtime.js，
// 由面板 fetch 後放進 iframe srcdoc（沙盒沒有網路，不能 <script src>）。
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  publicDir: false,
  define: { 'process.env.NODE_ENV': '"production"' },
  build: {
    outDir: 'public',
    emptyOutDir: false,
    copyPublicDir: false,
    assetsInlineLimit: () => true,
    minify: true,
    lib: {
      entry: 'src/canvas/runtime/main.tsx',
      formats: ['iife'],
      name: 'KunsuCanvasRuntime',
      fileName: () => 'canvas-runtime.js',
    },
  },
})
