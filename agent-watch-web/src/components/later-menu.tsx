import '#components/later-today.css'

import { Button } from '@fohte/ui/button'

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
  onSelect: (preset: LaterPreset) => void
  onBackNow: () => void
  onClose: () => void
}

export function LaterMenu({
  sessionTitle,
  isLater,
  laterTimes,
  laterTimesError,
  onSelect,
  onBackNow,
  onClose,
}: LaterMenuProps) {
  return (
    <div className="later-menu-overlay" onClick={onClose}>
      <section
        aria-label={`Later: ${sessionTitle}`}
        className="later-menu"
        role="menu"
        onClick={(event) => {
          event.stopPropagation()
        }}
      >
        <p className="later-menu-title">Later: {sessionTitle}</p>
        {LATER_PRESET_OPTIONS.map((option) => (
          <Button
            key={option.preset}
            type="button"
            variant="plain"
            role="menuitem"
            disabled={
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
            <kbd>{option.key}</kbd>
          </Button>
        ))}
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
            className="later-menu-option later-menu-reset cursor-pointer"
            onClick={onBackNow}
          >
            Back now
          </Button>
        ) : null}
        <p className="later-menu-note">
          New activity in the session also brings it back.
        </p>
      </section>
    </div>
  )
}
