import '#components/later-today.css'

import { Button } from '@fohte/ui/button'
import { useEffect, useRef } from 'react'

import { useDismissableLayer } from '#components/use-dismissable-layer'
import { useFocusTrap } from '#components/use-focus-trap'
import {
  LATER_PRESET_OPTIONS,
  type LaterPreset,
  type LaterPresetTimes,
} from '#session-navigation'

type LaterMenuProps = {
  sessionTitle: string
  isLater: boolean
  laterTimes: LaterPresetTimes | null
  laterTimesError: boolean
  isSubmitting: boolean
  onSelect: (preset: LaterPreset) => void
  onBackNow: () => void
  onClose: () => void
  isMobile?: boolean
}

export function LaterMenu({
  sessionTitle,
  isLater,
  laterTimes,
  laterTimesError,
  isSubmitting,
  onSelect,
  onBackNow,
  onClose,
  isMobile = false,
}: LaterMenuProps) {
  const menuRef = useRef<HTMLElement>(null)
  useFocusTrap({
    containerRef: menuRef,
    initialFocusRef: menuRef,
    isActive: isMobile,
  })
  useDismissableLayer(
    true,
    (open) => {
      if (!open && !isSubmitting) onClose()
    },
    [menuRef],
  )
  useEffect(() => {
    if (!isMobile) return
    const handleEscape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented || isSubmitting) {
        return
      }
      event.preventDefault()
      onClose()
    }
    window.addEventListener('keydown', handleEscape)
    return () => {
      window.removeEventListener('keydown', handleEscape)
    }
  }, [isMobile, isSubmitting, onClose])

  return (
    <div className="later-menu-overlay">
      <section
        aria-label={isMobile ? 'Later' : `Later: ${sessionTitle}`}
        aria-modal={isMobile || undefined}
        className={`later-menu${isMobile ? ' later-menu--mobile' : ''}`}
        role={isMobile ? 'dialog' : 'menu'}
        ref={menuRef}
        tabIndex={isMobile ? -1 : undefined}
      >
        {isMobile ? (
          <>
            <h2>Later</h2>
            <p className="later-menu-mobile-session-title">{sessionTitle}</p>
          </>
        ) : (
          <p className="later-menu-title">
            Later:{' '}
            <span className="later-menu-session-title">{sessionTitle}</span>
          </p>
        )}
        <div
          className="later-menu-options"
          aria-label={isMobile ? 'Bring it back' : undefined}
          role={isMobile ? 'menu' : undefined}
        >
          {LATER_PRESET_OPTIONS.map((option) => (
            <Button
              key={option.preset}
              type="button"
              variant="plain"
              role="menuitem"
              disabled={
                isSubmitting ||
                laterTimes === null ||
                (option.preset !== 'until_it_changes' &&
                  laterTimes[option.preset].deadline === null)
              }
              className="later-menu-option cursor-pointer"
              onClick={() => {
                onSelect(option.preset)
              }}
            >
              <span>{option.label}</span>
              <span className="later-menu-when">
                {laterTimes?.[option.preset].when ??
                  (laterTimesError ? 'Unavailable' : 'Loading…')}
              </span>
              {isMobile ? null : <kbd>{option.key}</kbd>}
            </Button>
          ))}
        </div>
        {laterTimesError ? (
          <p className="later-menu-error" role="alert">
            Could not load return times.
          </p>
        ) : null}
        {isLater ? (
          <Button
            type="button"
            variant="plain"
            role="menuitem"
            aria-label={`Back now: ${sessionTitle}`}
            disabled={isSubmitting}
            className="later-menu-option later-menu-reset cursor-pointer"
            onClick={onBackNow}
          >
            Back now
          </Button>
        ) : null}
        <div className="later-menu-divider" aria-hidden="true" />
        <p className="later-menu-note">
          New activity in the session also brings it back.
        </p>
        {isMobile ? (
          <Button
            type="button"
            variant="plain"
            className="later-menu-cancel"
            disabled={isSubmitting}
            onClick={onClose}
          >
            Cancel
          </Button>
        ) : null}
      </section>
    </div>
  )
}
