import '#components/agent-watch-nav.css'

import { Button } from '@fohte/ui/button'
import { Chip } from '@fohte/ui/chip'

import { cn } from '#lib/utils'

type View = 'inbox' | 'board'
type PmProject = {
  name: string
  status: 'Review' | 'Working' | 'Idle'
  needsActionCount: number
  sessionId: string | null
}

type AgentWatchNavProps = {
  activeView: View
  needsActionCount: number
  pmProjects: readonly PmProject[]
  onViewChange: (view: View) => void
  onSelectSession: (id: string) => void
}

export function AgentWatchNav({
  activeView,
  needsActionCount,
  pmProjects,
  onViewChange,
  onSelectSession,
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
      <section className="pm-project-list" aria-labelledby="pm-projects-title">
        <h2 id="pm-projects-title">PM</h2>
        <ul>
          {pmProjects.map((project) => (
            <li key={project.name}>
              <Button
                type="button"
                variant="plain"
                disabled={project.sessionId === null}
                className={cn(
                  'pm-project grid w-full',
                  'pm-project-' + project.status.toLowerCase(),
                  project.needsActionCount > 0 && 'has-needs-action',
                  project.sessionId !== null &&
                    'cursor-pointer hover:text-foreground',
                )}
                aria-label={
                  project.sessionId === null
                    ? `${project.name}: no PM session`
                    : `Open ${project.name} PM session in Inbox`
                }
                onClick={() => {
                  if (project.sessionId !== null) {
                    onSelectSession(project.sessionId)
                  }
                }}
              >
                <span className="pm-project-dot" aria-hidden="true" />
                <span className="pm-project-name">{project.name}</span>
                <span className="pm-project-status">{project.status}</span>
              </Button>
            </li>
          ))}
        </ul>
      </section>
    </nav>
  )
}
