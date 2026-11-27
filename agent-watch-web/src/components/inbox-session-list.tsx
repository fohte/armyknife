import '#components/inbox-session-list.css'

import { InboxSessionListItem } from '#inbox-session-list-item'
import type { Session } from '#types'

type InboxSessionListProps = {
  sessions: readonly Session[]
  selectedId: string | null
  onSelect: (id: string) => void
  isMobile: boolean
  groupByProject?: boolean
  className?: string
  onLater?: ((id: string) => void) | undefined
  onCloseSession?: ((id: string) => void) | undefined
  onOpenActions?: ((session: Session) => void) | undefined
}

export function InboxSessionList({
  sessions,
  selectedId,
  onSelect,
  isMobile,
  groupByProject = false,
  className = '',
  onLater,
  onCloseSession,
  onOpenActions,
}: InboxSessionListProps) {
  const renderRows = (items: readonly Session[]) =>
    items.map((session) => (
      <InboxSessionListItem
        key={session.id}
        session={session}
        selectedId={selectedId}
        onSelect={onSelect}
        isMobile={isMobile}
        onLater={onLater}
        onCloseSession={onCloseSession}
        onOpenActions={onOpenActions}
      />
    ))

  if (!groupByProject) {
    return (
      <ul className={`session-list ${className}`}>{renderRows(sessions)}</ul>
    )
  }

  const projects = new Map<string, Session[]>()
  for (const session of sessions) {
    const projectSessions = projects.get(session.project) ?? []
    projectSessions.push(session)
    projects.set(session.project, projectSessions)
  }

  return (
    <div className={`session-project-groups ${className}`}>
      {Array.from(projects, ([project, projectSessions]) => (
        <section className="session-project-group" key={project}>
          <h3 className="inbox-project-heading">{project}</h3>
          <ul className="session-list">{renderRows(projectSessions)}</ul>
        </section>
      ))}
    </div>
  )
}
