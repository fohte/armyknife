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
  mobileTabPresentation?: boolean
  previewMessage?: string
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
  mobileTabPresentation = false,
  previewMessage,
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
      <header
        className={`web-ui-pane-header${mobilePresentation ? ' is-mobile-fullscreen' : ''}${mobileTabPresentation ? ' is-mobile-tab' : ''}`}
      >
        {mobilePresentation || mobileTabPresentation ? (
          <>
            <div className="web-ui-pane-mobile-heading">
              <span className="web-ui-pane-title">{name}</span>
              <span className="web-ui-pane-mobile-url">{targetUrl ?? url}</span>
            </div>
            <div className="web-ui-pane-mobile-actions">
              {targetUrl !== null ? (
                <a
                  className="web-ui-pane-open-new-tab"
                  href={targetUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  aria-label="Open in a new tab"
                >
                  <svg
                    width="18"
                    height="18"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    aria-hidden="true"
                  >
                    <path d="M15 3h6v6" />
                    <path d="M10 14 21 3" />
                    <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
                  </svg>
                </a>
              ) : null}
              <Button
                type="button"
                variant="plain"
                aria-label="Close web UI"
                onClick={onClose}
              >
                <svg
                  width="22"
                  height="22"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden="true"
                >
                  <path d="M18 6 6 18" />
                  <path d="m6 6 12 12" />
                </svg>
              </Button>
            </div>
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
        {mobilePresentation || mobileTabPresentation ? null : (
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
      {previewMessage !== undefined ? (
        <div className="web-ui-pane-preview">{previewMessage}</div>
      ) : targetUrl !== null ? (
        <iframe
          key={fullscreenRequest}
          title={`${name} web interface`}
          src={targetUrl}
        />
      ) : (
        <p className="web-ui-pane-unavailable" role="status">
          This interface is unavailable outside this machine. Local URL: {url}
        </p>
      )}
    </aside>
  )
}
