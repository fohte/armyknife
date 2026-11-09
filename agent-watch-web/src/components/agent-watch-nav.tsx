import { cn } from '#lib/utils'

type View = 'inbox' | 'board'

type AgentWatchNavProps = {
  activeView: View
  needsActionCount: number
  onViewChange: (view: View) => void
}

export function AgentWatchNav({
  activeView,
  needsActionCount,
  onViewChange,
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
        <button
          type="button"
          aria-pressed={activeView === 'inbox'}
          className={cn(
            'view-switch-button',
            activeView === 'inbox' && 'is-active',
          )}
          onClick={() => {
            onViewChange('inbox')
          }}
        >
          <span>Inbox</span>
          <span className="view-count">{needsActionCount}</span>
        </button>
        <button
          type="button"
          aria-pressed={activeView === 'board'}
          className={cn(
            'view-switch-button',
            activeView === 'board' && 'is-active',
          )}
          onClick={() => {
            onViewChange('board')
          }}
        >
          Board
        </button>
      </div>
    </nav>
  )
}
