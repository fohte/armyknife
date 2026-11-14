import '#components/completed-items-toggle.css'

type CompletedItemsToggleProps = {
  count: number
  expanded: boolean
  onToggle: () => void
}

export function CompletedItemsToggle({
  count,
  expanded,
  onToggle,
}: CompletedItemsToggleProps) {
  return (
    <button
      className="completed-items-toggle completed-items-toggle--linked-task-menu session-task-links-completed-toggle"
      type="button"
      aria-expanded={expanded}
      onClick={onToggle}
    >
      {expanded ? 'Hide' : 'Show'} {String(count)} completed
    </button>
  )
}
