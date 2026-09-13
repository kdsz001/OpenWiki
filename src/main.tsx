import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './i18n'
import './index.css'
import App from './App.tsx'
import SpotlightView from './features/spotlight/SpotlightView.tsx'
import BubbleEntry from './components/BubbleEntry.tsx'
import FootballField from './features/football/FootballField.tsx'

/** Determine which component to render based on the URL path. */
const pathname = window.location.pathname

// Bubble, spotlight and football field windows need fully transparent backgrounds
if (pathname === '/bubble' || pathname === '/spotlight' || pathname === '/football-field') {
  document.documentElement.classList.add('transparent-window')
}

const RootComponent =
  pathname === '/spotlight' ? SpotlightView
  : pathname === '/bubble' ? BubbleEntry
  : pathname === '/football-field' ? FootballField
  : App

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RootComponent />
  </StrictMode>,
)
