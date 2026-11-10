import '#components/later-today.css'

import { Button } from '@fohte/ui/button'

import { LATER_PRESET_OPTIONS, type LaterPreset } from '#session-navigation'

type LaterMenuProps = {
  sessionTitle: string
  onSelect: (preset: LaterPreset) => void
  onClose: () => void
}

export function LaterMenu({ sessionTitle, onSelect, onClose }: LaterMenuProps) {
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
            className="later-menu-option cursor-pointer"
            onClick={() => {
              onSelect(option.preset)
            }}
          >
            <span>{option.label}</span>
            <span className="later-menu-when">{option.when}</span>
            <kbd>{option.key}</kbd>
          </Button>
        ))}
        <p className="later-menu-note">
          New activity in the session also brings it back.
        </p>
      </section>
    </div>
  )
}
