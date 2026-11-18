import '#components/vibe-panel.css'

import { Button } from '@fohte/ui/button'

import type { VibeMode } from '#types'
import { vibeModeChange, vibeModeLabel, vibeModeReason } from '#vibe-mode'

type VibePanelProps = {
  mode: VibeMode
  repo: string
  error?: string | null
  pending?: boolean
  isMobile?: boolean
  onClose: () => void
  onChange: () => void
}

export function VibePanel({
  mode,
  repo,
  error = null,
  pending = false,
  isMobile = false,
  onClose,
  onChange,
}: VibePanelProps) {
  const change = vibeModeChange(mode)
  const unavailable = mode.mode === 'repo_disabled'

  return (
    <section
      className={`vibe-panel${isMobile ? ' vibe-panel--mobile' : ''}`}
      role="dialog"
      aria-label="Vibe mode"
      aria-modal={isMobile || undefined}
      onClick={(event) => {
        event.stopPropagation()
      }}
    >
      <h2 className="vibe-panel-title">{vibeModeLabel(mode)}</h2>
      <p className="vibe-panel-reason">{vibeModeReason(mode, repo)}</p>
      {unavailable ? null : (
        <div className="vibe-panel-message">
          <span className="vibe-panel-message-label">Message to send</span>
          <blockquote>{change.message}</blockquote>
        </div>
      )}
      {error === null ? null : (
        <p className="vibe-panel-error" role="alert">
          {error}
        </p>
      )}
      <div className="vibe-panel-actions">
        <Button
          className="vibe-panel-close cursor-pointer"
          type="button"
          variant="plain"
          disabled={pending}
          onClick={onClose}
        >
          Close
        </Button>
        {unavailable ? null : (
          <Button
            className="vibe-panel-change cursor-pointer"
            type="button"
            variant="plain"
            disabled={pending}
            onClick={onChange}
          >
            {pending ? 'Sending…' : change.actionLabel}
          </Button>
        )}
      </div>
    </section>
  )
}
