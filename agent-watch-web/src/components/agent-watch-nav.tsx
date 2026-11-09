import { Button } from '@fohte/ui/button'
import { Chip } from '@fohte/ui/chip'

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
    </nav>
  )
}
