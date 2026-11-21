import '#components/web-ui-pane.css'

import { Button } from '@fohte/ui/button'
import { useEffect, useRef, useState } from 'react'

import { webUiTargetUrl } from '#components/web-ui-target-url'

type WebUiPaneProps = {
  name: string
  url: string
  exposedUrl?: string | null
  hostname?: string
  onClose: () => void
  defaultFullscreen?: boolean
  fullscreenRequest?: number
  targetUrlOverride?: string | null
  mobilePresentation?: boolean
}

export function WebUiPane({
  name,
  url,
  exposedUrl,
  hostname,
  onClose,
  defaultFullscreen = false,
  fullscreenRequest = 0,
  targetUrlOverride,
  mobilePresentation = false,
}: WebUiPaneProps) {
  const [isFullscreen, setIsFullscreen] = useState(
    defaultFullscreen || fullscreenRequest > 0,
  )
  const previousFullscreenRequest = useRef(fullscreenRequest)
  const targetUrl =
    targetUrlOverride === undefined
      ? webUiTargetUrl(
          { url, exposedUrl: exposedUrl ?? null },
          hostname ?? location.hostname,
        )
      : targetUrlOverride

  useEffect(() => {
    if (previousFullscreenRequest.current === fullscreenRequest) return
    previousFullscreenRequest.current = fullscreenRequest
    if (fullscreenRequest > 0) setIsFullscreen(true)
  }, [fullscreenRequest])

  return (
    <aside
      className={`web-ui-pane${isFullscreen ? ' is-fullscreen' : ''}${mobilePresentation ? ' is-mobile-fullscreen' : ''}`}
      aria-label={name}
      role={mobilePresentation ? 'dialog' : undefined}
      aria-modal={mobilePresentation || undefined}
      id="web-ui-pane"
    >
      <header className="web-ui-pane-header">
        {mobilePresentation ? (
          <>
            <Button
              type="button"
              variant="plain"
              aria-label="Back to conversation"
              onClick={onClose}
            >
              ←
            </Button>
            <span className="web-ui-pane-title">{name}</span>
          </>
        ) : (
          <>
            <span className="web-ui-pane-title">{name}</span>
            {targetUrl !== null ? (
              <a href={targetUrl} target="_blank" rel="noreferrer">
                {targetUrl}
              </a>
            ) : (
              <span>{url}</span>
            )}
          </>
        )}
        {mobilePresentation ? null : (
          <>
            <Button
              type="button"
              variant="plain"
              aria-label={
                isFullscreen
                  ? `Exit ${name} fullscreen`
                  : `Show ${name} fullscreen`
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
          </>
        )}
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
