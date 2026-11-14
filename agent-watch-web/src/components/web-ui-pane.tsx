import '#components/web-ui-pane.css'

import { Button } from '@fohte/ui/button'
import { useState } from 'react'

import { webUiTargetUrl } from '#components/web-ui-target-url'

type WebUiPaneProps = {
  name: string
  url: string
  exposedUrl?: string | null
  hostname?: string
  onClose: () => void
  defaultFullscreen?: boolean
}

export function WebUiPane({
  name,
  url,
  exposedUrl,
  hostname,
  onClose,
  defaultFullscreen = false,
}: WebUiPaneProps) {
  const [isFullscreen, setIsFullscreen] = useState(defaultFullscreen)
  const targetUrl = webUiTargetUrl(
    { url, exposedUrl: exposedUrl ?? null },
    hostname ?? location.hostname,
  )

  return (
    <aside
      className={`web-ui-pane${isFullscreen ? ' is-fullscreen' : ''}`}
      aria-label={name}
      id="web-ui-pane"
    >
      <header className="web-ui-pane-header">
        <span className="web-ui-pane-title">{name}</span>
        {targetUrl !== null ? (
          <a href={targetUrl} target="_blank" rel="noreferrer">
            {targetUrl}
          </a>
        ) : (
          <span>{url}</span>
        )}
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
      {targetUrl !== null ? (
        <iframe title={`${name} web interface`} src={targetUrl} />
      ) : (
        <p className="web-ui-pane-unavailable" role="status">
          This interface is unavailable outside this machine. Local URL: {url}
        </p>
      )}
    </aside>
  )
}
