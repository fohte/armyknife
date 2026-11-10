import '#components/later-today.css'

import { Button } from '@fohte/ui/button'

import type { LaterPreset } from '#session-navigation'

const options: readonly {
  preset: LaterPreset
  label: string
  when: string
  key: string
}[] = [
  { preset: 'one_hour', label: 'In 1 hour', when: '1h', key: '1' },
  { preset: 'this_evening', label: 'This evening', when: '19:00', key: '2' },
  { preset: 'tomorrow', label: 'Tomorrow', when: '9:00', key: '3' },
  { preset: 'next_week', label: 'Next week', when: 'Mon 11:00', key: '4' },
  {
    preset: 'until_it_changes',
    label: 'Until it changes',
    when: 'next activity',
    key: '5',
  },
]

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
        {options.map((option) => (
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
