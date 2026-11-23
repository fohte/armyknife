type MobileTodoLaterCountProps = {
  count: number
}

export function MobileTodoLaterCount({ count }: MobileTodoLaterCountProps) {
  return count > 0 ? (
    <span className="mobile-todo-later-count">{count} to-do later</span>
  ) : null
}
