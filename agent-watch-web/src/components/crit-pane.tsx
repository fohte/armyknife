import '#components/crit-pane.css'

import { Button } from '@fohte/ui/button'
import { useState } from 'react'

import { critRelayUrl } from '#components/crit-relay-url'

type CritPaneProps = {
  url: string
  onClose: () => void
  defaultFullscreen?: boolean
}

export function CritPane({
  url,
  onClose,
  defaultFullscreen = false,
}: CritPaneProps) {
  const [isFullscreen, setIsFullscreen] = useState(defaultFullscreen)
  const relayUrl = critRelayUrl(url)

  return (
    <aside
      className={`crit-pane${isFullscreen ? ' is-fullscreen' : ''}`}
      aria-label="crit"
      id="crit-pane"
    >
      <header className="crit-pane-header">
        <span className="crit-pane-title">crit</span>
        <a href={relayUrl} target="_blank" rel="noreferrer">
          {url}
        </a>
        <Button
          type="button"
          variant="plain"
          aria-label={
            isFullscreen ? 'Exit crit fullscreen' : 'Show crit fullscreen'
          }
          aria-pressed={isFullscreen}
          onClick={() => {
            setIsFullscreen((current) => !current)
          }}
        >
          {isFullscreen ? '↙' : '⛶'}
        </Button>
        <Button
          type="button"
          variant="plain"
          aria-label="Close crit"
          onClick={onClose}
        >
          ×
        </Button>
      </header>
      <iframe title="Crit review" src={relayUrl} />
    </aside>
  )
}
