import { Button } from '@fohte/ui/button'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { useState } from 'react'

import { InboxSessionListItem } from '#inbox-session-list-item'
import type { Session } from '#types'

type InboxTodoLaterSectionProps = {
  sessions: readonly Session[]
  selectedId: string | null
  onSelect: (id: string) => void
  isMobile: boolean
  expanded?: boolean
  onExpandedChange?: (expanded: boolean) => void
}

export function InboxTodoLaterSection({
  sessions,
  selectedId,
  onSelect,
  isMobile,
  expanded: controlledExpanded,
  onExpandedChange,
}: InboxTodoLaterSectionProps) {
  const [localExpanded, setLocalExpanded] = useState(false)
  const expanded = controlledExpanded ?? localExpanded
  const todoLaterSessions = sessions.filter(
    (session) => (session.todoLaterCount ?? 0) > 0,
  )
  const firstTodoLaterSession = todoLaterSessions[0]

  if (firstTodoLaterSession === undefined) return null

  const hasMultipleTodoLaterSessions = todoLaterSessions.length > 1
  const todoLaterCount = todoLaterSessions.reduce(
    (count, session) => count + (session.todoLaterCount ?? 0),
    0,
  )

  return (
    <>
      <Button
        type="button"
        variant="plain"
        className="todo-later-row cursor-pointer"
        aria-expanded={hasMultipleTodoLaterSessions ? expanded : undefined}
        onClick={() => {
          if (hasMultipleTodoLaterSessions) {
            const nextExpanded = !expanded
            if (onExpandedChange !== undefined) {
              onExpandedChange(nextExpanded)
            } else {
              setLocalExpanded(nextExpanded)
            }
          } else {
            onSelect(firstTodoLaterSession.id)
          }
        }}
      >
        <span className="todo-later-label">To-do later</span>
        <span className="todo-later-count">{todoLaterCount}</span>
        <span className="todo-later-title">
          {firstTodoLaterSession.todoLaterTitle}
        </span>
        {hasMultipleTodoLaterSessions && expanded ? (
          <ChevronDown aria-hidden="true" className="todo-later-chevron" />
        ) : (
          <ChevronRight aria-hidden="true" className="todo-later-chevron" />
        )}
      </Button>
      {hasMultipleTodoLaterSessions && expanded ? (
        <ul
          className="session-list todo-later-session-list"
          aria-label="To-do later sessions"
        >
          {todoLaterSessions.map((session) => (
            <InboxSessionListItem
              key={session.id}
              session={session}
              selectedId={selectedId}
              onSelect={onSelect}
              isMobile={isMobile}
            />
          ))}
        </ul>
      ) : null}
    </>
  )
}
