import '#components/agent-watch-nav.css'
import '#components/later-today.css'

import { Button } from '@fohte/ui/button'
import { Chip } from '@fohte/ui/chip'
import { Filter, PanelLeftClose, PanelLeftOpen, Search, X } from 'lucide-react'

import type { WorkTypeNavGroup } from '#board-model'
import type { CommandPaletteShortcut } from '#command-palette-shortcut'
import { cn } from '#lib/utils'
import type { SavedInboxView } from '#saved-inbox-views'
import type { Session } from '#types'

type View = 'inbox' | 'board'
type SavedViewNavItem = SavedInboxView & {
  count: number
  countTone: 'needs-action' | 'error' | null
}
type AgentWatchNavProps = {
  activeView: View
  paletteShortcut: CommandPaletteShortcut
  needsActionCount: number
  savedViews: readonly SavedViewNavItem[]
  selectedSavedViewId: string | null
  workTypeNavGroups: readonly WorkTypeNavGroup[]
  onViewChange: (view: View) => void
  onSelectSavedView: (id: string) => void
  onDeleteSavedView: (id: string) => void
  todaySessions: readonly Session[]
  onSelectToday: (id: string) => void
  onSelectSession: (id: string) => void
  onOpenPalette: () => void
  inboxCollapsed?: boolean
  onToggleInbox?: () => void
}

export function AgentWatchNav({
  activeView,
  paletteShortcut,
  needsActionCount,
  savedViews,
  selectedSavedViewId,
  workTypeNavGroups,
  onViewChange,
  onSelectSavedView,
  onDeleteSavedView,
  todaySessions,
  onSelectToday,
  onSelectSession,
  onOpenPalette,
  inboxCollapsed = false,
  onToggleInbox = () => undefined,
}: AgentWatchNavProps) {
  return (
    <nav className="agent-watch-nav" aria-label="Main navigation">
      <div className="brand-row">
        <span className="brand-mark" aria-hidden="true">
          &gt;
        </span>
        <span className="brand-name">agent watch</span>
        <div className="agent-watch-command">
          <Search aria-hidden="true" size={12} />
          <button
            aria-label={`Jump to with ${paletteShortcut.ariaLabel}`}
            className="agent-watch-command-key"
            onClick={onOpenPalette}
            type="button"
          >
            {paletteShortcut.label}
          </button>
        </div>
        <button
          aria-label={inboxCollapsed ? 'Show Inbox' : 'Hide Inbox'}
          aria-pressed={inboxCollapsed}
          className={`agent-watch-inbox-toggle${inboxCollapsed ? ' is-active' : ''}`}
          onClick={() => {
            onToggleInbox()
          }}
          title={inboxCollapsed ? 'Show Inbox' : 'Hide Inbox'}
          type="button"
        >
          {inboxCollapsed ? (
            <PanelLeftOpen aria-hidden="true" size={13} />
          ) : (
            <PanelLeftClose aria-hidden="true" size={13} />
          )}
        </button>
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
      {savedViews.length > 0 ? (
        <section
          className="saved-views-nav"
          aria-labelledby="saved-views-title"
        >
          <h2 id="saved-views-title">VIEWS</h2>
          <ul>
            {savedViews.map((savedView) => (
              <li key={savedView.id} className="saved-view-item">
                <Button
                  type="button"
                  variant="plain"
                  aria-pressed={selectedSavedViewId === savedView.id}
                  className={cn(
                    'saved-view-link cursor-pointer',
                    selectedSavedViewId === savedView.id && 'is-active',
                  )}
                  onClick={() => {
                    onSelectSavedView(savedView.id)
                  }}
                >
                  <Filter aria-hidden="true" size={13} />
                  <span className="saved-view-name">{savedView.name}</span>
                  <span
                    className={cn(
                      'saved-view-count',
                      savedView.countTone !== null &&
                        `is-${savedView.countTone}`,
                    )}
                  >
                    {savedView.count}
                  </span>
                </Button>
                <Button
                  type="button"
                  variant="plain"
                  aria-label={`Delete ${savedView.name} view`}
                  className="saved-view-delete cursor-pointer"
                  onClick={() => {
                    onDeleteSavedView(savedView.id)
                  }}
                >
                  <X aria-hidden="true" size={13} />
                </Button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
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
      {workTypeNavGroups.map((group, index) => (
        <section
          key={group.label}
          className="work-type-nav"
          aria-labelledby={`work-type-nav-title-${String(index)}`}
        >
          <h2 id={`work-type-nav-title-${String(index)}`}>{group.label}</h2>
          <ul>
            {group.projects.map((project) => (
              <li key={project.name}>
                <Button
                  type="button"
                  variant="plain"
                  className={cn(
                    'work-type-project grid w-full',
                    'work-type-project-' + project.status.toLowerCase(),
                    project.needsActionCount > 0 && 'has-needs-action',
                    'cursor-pointer hover:text-foreground',
                  )}
                  aria-label={`Open ${project.name} ${group.label} session in Inbox`}
                  onClick={() => {
                    onSelectSession(project.sessionId)
                  }}
                >
                  <span className="work-type-project-dot" aria-hidden="true" />
                  <span className="work-type-project-name">{project.name}</span>
                  <span className="work-type-project-status">
                    {project.status}
                  </span>
                </Button>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </nav>
  )
}
