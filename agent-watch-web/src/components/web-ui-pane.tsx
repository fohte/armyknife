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
  size?: 'desktop' | 'phone'
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
  size = 'desktop',
  previewMessage,
}: WebUiPaneProps) {
  const isPhone = size === 'phone'
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

  const fullscreenButton = (
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
      {isFullscreen ? (
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <path d="m14 10 7-7" />
          <path d="M20 10h-6V4" />
          <path d="m3 21 7-7" />
          <path d="M4 14h6v6" />
        </svg>
      ) : (
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <path d="M15 3h6v6" />
          <path d="m21 3-7 7" />
          <path d="m3 21 7-7" />
          <path d="M9 21H3v-6" />
        </svg>
      )}
    </Button>
  )

  useEffect(() => {
    if (previousFullscreenRequest.current === fullscreenRequest) return
    previousFullscreenRequest.current = fullscreenRequest
    if (fullscreenRequest > 0) setIsFullscreen(true)
  }, [fullscreenRequest])

  return (
    <aside
      className={`web-ui-pane${isFullscreen ? ' is-fullscreen' : ''}`}
      data-size={size}
      aria-label={name}
      role={isPhone && isFullscreen ? 'dialog' : undefined}
      aria-modal={(isPhone && isFullscreen) || undefined}
      id="web-ui-pane"
    >
      <header className="web-ui-pane-header">
        <div className="web-ui-pane-heading">
          <span className="web-ui-pane-title">{name}</span>
          <span className="web-ui-pane-url">{targetUrl ?? url}</span>
        </div>
        {targetUrl !== null ? (
          <a
            className="web-ui-pane-open-new-tab"
            href={targetUrl}
            target="_blank"
            rel="noopener noreferrer"
            aria-label="Open in a new tab"
            title="Open in a new tab"
          >
            <svg
              width={isPhone ? '18' : '16'}
              height={isPhone ? '18' : '16'}
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
        {isPhone ? null : fullscreenButton}
        <Button
          type="button"
          variant="plain"
          aria-label="Close web UI"
          title="Close web UI"
          onClick={onClose}
        >
          <svg
            width={isPhone ? '22' : '16'}
            height={isPhone ? '22' : '16'}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth={isPhone ? '2.5' : '2'}
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M18 6 6 18" />
            <path d="m6 6 12 12" />
          </svg>
        </Button>
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
