import '#components/later-today.css'

import { Button } from '@fohte/ui/button'
import { Chip } from '@fohte/ui/chip'

import { cn } from '#lib/utils'
import type { Session } from '#types'

type View = 'inbox' | 'board'

type AgentWatchNavProps = {
  activeView: View
  needsActionCount: number
  onViewChange: (view: View) => void
  todaySessions: readonly Session[]
  onSelectToday: (id: string) => void
}

export function AgentWatchNav({
  activeView,
  needsActionCount,
  onViewChange,
  todaySessions,
  onSelectToday,
}: AgentWatchNavProps) {
  return (
    <nav className="agent-watch-nav" aria-label="Main navigation">
      <div className="brand-row">
        <span className="brand-mark" aria-hidden="true">
          &gt;
        </span>
        <span className="brand-name">agent watch</span>
      </div>
      <div className="view-switch" role="group" aria-label="Views">
        <Button
          type="button"
          aria-pressed={activeView === 'inbox'}
          variant="plain"
          className={cn(
            'view-switch-button cursor-pointer',
            activeView === 'inbox' && 'is-active',
          )}
          onClick={() => {
            onViewChange('inbox')
          }}
        >
          <span>Inbox</span>
          <Chip data-view-count="" tone="strong">
            {needsActionCount}
          </Chip>
        </Button>
        <Button
          type="button"
          aria-pressed={activeView === 'board'}
          variant="plain"
          className={cn(
            'view-switch-button cursor-pointer',
            activeView === 'board' && 'is-active',
          )}
          onClick={() => {
            onViewChange('board')
          }}
        >
          Board
        </Button>
      </div>
      <section className="today-nav" aria-labelledby="today-nav-title">
        <h2 id="today-nav-title">TODAY</h2>
        {todaySessions.map((session) => (
          <div key={session.id} className="today-nav-item">
            <Button
              type="button"
              variant="plain"
              className="today-nav-link cursor-pointer"
              onClick={() => {
                onSelectToday(session.id)
              }}
            >
              <span className="today-star" aria-hidden="true">
                ★
              </span>
              <span
                className={cn(
                  'today-session-title',
                  session.category === 'needs_action' && 'is-needs-action',
                )}
              >
                {session.title}
              </span>
              {session.category === 'needs_action' ? (
                <span
                  className={`today-indicator kind-${(session.kind ?? '').toLowerCase()}`}
                  aria-hidden="true"
                />
              ) : (
                <span className="today-start">Start</span>
              )}
            </Button>
            {session.task !== null ? (
              <span className="today-session-meta">
                #{String(session.task.number)} · {session.repo}
              </span>
            ) : null}
          </div>
        ))}
      </section>
    </nav>
  )
}
