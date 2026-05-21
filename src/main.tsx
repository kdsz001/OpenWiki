import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './i18n'
import './index.css'
import App from './App.tsx'
import SpotlightView from './features/spotlight/SpotlightView.tsx'
import BubbleView from './components/BubbleView.tsx'

/** Determine which component to render based on the URL path. */
const pathname = window.location.pathname

// Bubble and spotlight windows need fully transparent backgrounds
if (pathname === '/bubble' || pathname === '/spotlight') {
  document.documentElement.classList.add('transparent-window')
}


const RootComponent =
  pathname === '/spotlight' ? SpotlightView
  : pathname === '/bubble' ? BubbleView
  : App

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RootComponent />
  </StrictMode>,
)
