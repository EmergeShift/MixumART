import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import App from './App'
import { SurfaceView } from './SurfaceView'
import './styles.css'

declare global {
  interface Window {
    __MIXUM_SURFACE__?: { surface: number; addr: string }
  }
}

const surface = window.__MIXUM_SURFACE__

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {surface ? <SurfaceView surface={surface.surface} addr={surface.addr} /> : <App />}
  </StrictMode>,
)
