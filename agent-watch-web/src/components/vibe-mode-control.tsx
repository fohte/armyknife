import '#components/vibe-panel.css'

import { useCallback, useEffect, useState } from 'react'

import { VibeIndicator } from '#components/vibe-indicator'
import { VibePanel } from '#components/vibe-panel'
import type { VibeMode } from '#types'
import { vibeModeChange, vibeModeIsOn, vibeModeLabel } from '#vibe-mode'
import { setVibeMode } from '#vibe-mode-api'

type VibeModeControlProps = {
  sessionId: string
  repo: string
  mode?: VibeMode
  isMobile?: boolean
  showTrigger?: boolean
  open?: boolean
  onOpenChange?: (open: boolean) => void
}

const DISABLED_MODE: VibeMode = { mode: 'disabled' }

export function VibeModeControl({
  sessionId,
  repo,
  mode = DISABLED_MODE,
  isMobile = false,
  showTrigger = !isMobile,
  open,
  onOpenChange,
}: VibeModeControlProps) {
  const [currentMode, setCurrentMode] = useState(mode)
  const [localOpen, setLocalOpen] = useState(false)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const isOpen = open ?? localOpen

  const setOpen = useCallback(
    (nextOpen: boolean) => {
      if (pending && !nextOpen) return
      if (open === undefined) setLocalOpen(nextOpen)
      onOpenChange?.(nextOpen)
      if (nextOpen) setError(null)
    },
    [onOpenChange, open, pending],
  )

  useEffect(() => {
    setCurrentMode(mode)
  }, [mode])

  useEffect(() => {
    if (!isOpen) return
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false)
    }
    window.addEventListener('keydown', closeOnEscape)
    return () => {
      window.removeEventListener('keydown', closeOnEscape)
    }
  }, [isOpen, setOpen])

  const changeMode = async () => {
    if (pending || currentMode.mode === 'repo_disabled') return
    setPending(true)
    setError(null)
    const change = vibeModeChange(currentMode)
    const result = await setVibeMode(sessionId, change.enabled)
    result.match(
      (resultingMode) => {
        setCurrentMode(resultingMode)
        setOpen(false)
      },
      (requestError) => {
        setError(requestError.message)
      },
    )
    setPending(false)
  }

  const panel = (
    <VibePanel
      mode={currentMode}
      repo={repo}
      isMobile={isMobile}
      error={error}
      pending={pending}
      onClose={() => {
        setOpen(false)
      }}
      onChange={() => {
        void changeMode()
      }}
    />
  )

  if (isMobile) {
    return isOpen ? (
      <div
        className="vibe-panel-backdrop"
        onClick={() => {
          setOpen(false)
        }}
        role="presentation"
      >
        {panel}
      </div>
    ) : null
  }

  return (
    <span className="vibe-control">
      {showTrigger ? (
        <button
          className={`vibe-control-trigger${vibeModeIsOn(currentMode) ? ' is-on' : ''}${isOpen ? ' is-open' : ''}`}
          type="button"
          aria-haspopup="dialog"
          aria-expanded={isOpen}
          aria-label={vibeModeLabel(currentMode)}
          title={vibeModeLabel(currentMode)}
          onClick={() => {
            setOpen(!isOpen)
          }}
        >
          <VibeIndicator mode={currentMode} size={12} />
          {vibeModeLabel(currentMode)}
        </button>
      ) : null}
      {isOpen ? <span className="vibe-control-popover">{panel}</span> : null}
    </span>
  )
}
