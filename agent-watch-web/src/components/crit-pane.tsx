import '#components/crit-pane.css'

import { Button } from '@fohte/ui/button'
import { useState } from 'react'

import { webUiRelayUrl } from '#components/crit-relay-url'

type CritPaneProps = {
  name: string
  url: string
  onClose: () => void
  defaultFullscreen?: boolean
}

export function CritPane({
  name,
  url,
  onClose,
  defaultFullscreen = false,
}: CritPaneProps) {
  const [isFullscreen, setIsFullscreen] = useState(defaultFullscreen)
  const relayUrl = webUiRelayUrl(url)

  return (
    <aside
      className={`crit-pane${isFullscreen ? ' is-fullscreen' : ''}`}
      aria-label={name}
      id="crit-pane"
    >
      <header className="crit-pane-header">
        <span className="crit-pane-title">{name}</span>
        <a href={relayUrl} target="_blank" rel="noreferrer">
          {url}
        </a>
        <Button
          type="button"
          variant="plain"
          aria-label={
            isFullscreen ? `Exit ${name} fullscreen` : `Show ${name} fullscreen`
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
          aria-label={`Close ${name}`}
          onClick={onClose}
        >
          ×
        </Button>
      </header>
      <iframe title={`${name} web interface`} src={relayUrl} />
    </aside>
  )
}
