import '#components/starred-session-toggle.css'

import { Button } from '@fohte/ui/button'

type StarredSessionToggleProps = {
  className: string
  size: 'desktop' | 'mobile'
  isStarred: boolean
  onToggleToday?: ((isStarred: boolean) => void) | undefined
}

export function StarredSessionToggle({
  className,
  size,
  isStarred,
  onToggleToday,
}: StarredSessionToggleProps) {
  const label = isStarred ? 'Remove from Starred' : 'Add to Starred'

  return (
    <Button
      className={`starred-session-toggle ${className}`}
      data-size={size}
      type="button"
      variant="plain"
      aria-label={label}
      aria-pressed={isStarred}
      title={label}
      onClick={() => {
        onToggleToday?.(!isStarred)
      }}
    >
      {isStarred ? '★' : '☆'}
    </Button>
  )
}
