import '#components/vibe-panel.css'

import { ConfirmDialog } from '#components/confirm-dialog'
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
    <ConfirmDialog
      className={`vibe-panel${isMobile ? ' vibe-panel--mobile' : ''}`}
      title={vibeModeLabel(mode)}
      titleClassName="vibe-panel-title"
      description={vibeModeReason(mode, repo)}
      confirmLabel={change.actionLabel}
      pendingLabel="Sending…"
      pending={pending}
      confirmDisabled={unavailable}
      hideConfirm={unavailable}
      cancelLabel="Close"
      cancelButtonClassName="vibe-panel-close"
      confirmButtonClassName="vibe-panel-change"
      dialogRole="dialog"
      presentation="inline"
      mobile={isMobile}
      onCancel={onClose}
      onConfirm={onChange}
    >
      {error === null ? null : (
        <p className="vibe-panel-error" role="alert">
          {error}
        </p>
      )}
    </ConfirmDialog>
  )
}
