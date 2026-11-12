import '#components/completed-items-toggle.css'

type CompletedItemsToggleProps = {
  count: number
  expanded: boolean
  onToggle: () => void
  placement: 'pull-request-list' | 'linked-task-menu'
}

const placementClasses = {
  'pull-request-list':
    'completed-items-toggle--pull-request-list pull-request-list-completed-toggle',
  'linked-task-menu':
    'completed-items-toggle--linked-task-menu session-task-links-completed-toggle',
} as const

export function CompletedItemsToggle({
  count,
  expanded,
  onToggle,
  placement,
}: CompletedItemsToggleProps) {
  return (
    <button
      className={`completed-items-toggle ${placementClasses[placement]}`}
      type="button"
      aria-expanded={expanded}
      onClick={onToggle}
    >
      {expanded ? 'Hide' : 'Show'} {String(count)} completed
    </button>
  )
}
