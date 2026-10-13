import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import '@fontsource/geist-sans/400.css'
import '@fontsource/geist-sans/500.css'
import '@fontsource/geist-sans/600.css'
import '@fontsource/geist-mono/400.css'
import './index.css'
import App from './App.tsx'
import { CanvasCheckPage } from './canvas/CanvasCheckPage'

// 證據畫布的註冊檢查視窗（隱藏）：只渲染候選題面板，不跑主 app。
const checkView = new URLSearchParams(window.location.search).get('view') === 'canvas-check'

createRoot(document.getElementById('root')!).render(
  <StrictMode>{checkView ? <CanvasCheckPage /> : <App />}</StrictMode>,
)
