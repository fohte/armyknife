import '#index.css'

import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { App } from '#App'
import { applyDesktopTitleBarMarker } from '#desktop-titlebar'

applyDesktopTitleBarMarker(navigator.userAgent, document.documentElement)

const rootElement = document.getElementById('root')

if (rootElement) {
  createRoot(rootElement).render(
    <StrictMode>
      <App />
    </StrictMode>,
  )
}
