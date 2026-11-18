import '#components/vibe-panel.css'

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { createPortal } from 'react-dom'

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

type PopoverPosition = {
  left: number
  top: number
  width: number
}

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
  const [popoverPosition, setPopoverPosition] =
    useState<PopoverPosition | null>(null)
  const controlRef = useRef<HTMLSpanElement>(null)
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

  useLayoutEffect(() => {
    if (!isOpen || isMobile) return

    const updatePosition = () => {
      const control = controlRef.current
      if (control === null) return

      const paneRect = control
        .closest('.resizable-columns-pane')
        ?.getBoundingClientRect()
      const leftBoundary = Math.max(16, paneRect?.left ?? 0)
      const rightBoundary = Math.min(
        window.innerWidth - 16,
        paneRect?.right ?? window.innerWidth,
      )
      const availableWidth = Math.max(0, rightBoundary - leftBoundary)
      const width = Math.min(360, availableWidth)
      const controlRect = control.getBoundingClientRect()

      const nextPosition = {
        left: Math.max(
          leftBoundary,
          Math.min(controlRect.left, rightBoundary - width),
        ),
        top: controlRect.bottom + 8,
        width,
      }
      setPopoverPosition((current) =>
        current?.left === nextPosition.left &&
        current.top === nextPosition.top &&
        current.width === nextPosition.width
          ? current
          : nextPosition,
      )
    }

    updatePosition()
    const resizeObserver = new ResizeObserver(updatePosition)
    const control = controlRef.current
    if (control !== null) {
      resizeObserver.observe(control)
      const pane = control.closest('.resizable-columns-pane')
      if (pane !== null) resizeObserver.observe(pane)
    }
    window.addEventListener('resize', updatePosition)
    window.addEventListener('scroll', updatePosition, true)
    return () => {
      resizeObserver.disconnect()
      window.removeEventListener('resize', updatePosition)
      window.removeEventListener('scroll', updatePosition, true)
    }
  }, [isMobile, isOpen])

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
    <>
      <span ref={controlRef} className="vibe-control">
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
      </span>
      {isOpen
        ? createPortal(
            <span
              className="vibe-control-popover"
              style={{
                left: `${String(popoverPosition?.left ?? 0)}px`,
                top: `${String(popoverPosition?.top ?? 0)}px`,
                visibility: popoverPosition === null ? 'hidden' : 'visible',
                width: `${String(popoverPosition?.width ?? 360)}px`,
              }}
            >
              {panel}
            </span>,
            document.body,
          )
        : null}
    </>
  )
}
