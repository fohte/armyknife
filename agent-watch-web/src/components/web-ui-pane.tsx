import '#components/web-ui-pane.css'

import { Button } from '@fohte/ui/button'
import { useState } from 'react'

import { webUiRelayUrl } from '#components/web-ui-relay-url'

type WebUiPaneProps = {
  name: string
  url: string
  onClose: () => void
  defaultFullscreen?: boolean
}

export function WebUiPane({
  name,
  url,
  onClose,
  defaultFullscreen = false,
}: WebUiPaneProps) {
  const [isFullscreen, setIsFullscreen] = useState(defaultFullscreen)
  const relayUrl = webUiRelayUrl(url)

  return (
    <aside
      className={`web-ui-pane${isFullscreen ? ' is-fullscreen' : ''}`}
      aria-label={name}
      id="web-ui-pane"
    >
      <header className="web-ui-pane-header">
        <span className="web-ui-pane-title">{name}</span>
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
