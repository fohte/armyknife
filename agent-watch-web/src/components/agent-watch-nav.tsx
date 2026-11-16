import '#components/agent-watch-nav.css'
import '#components/later-today.css'

import { Button } from '@fohte/ui/button'
import { Chip } from '@fohte/ui/chip'
import { Filter, PanelLeftClose, PanelLeftOpen, Search, X } from 'lucide-react'

import type { WorkTypeNavGroup } from '#board-model'
import type { CommandPaletteShortcut } from '#command-palette-shortcut'
import { SessionRow } from '#components/session-row'
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
  sessions?: readonly Session[]
  onSelectToday: (id: string) => void
  onSelectSession: (id: string) => void
  onOpenPalette: () => void
  isMobile?: boolean
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
  sessions = [],
  onSelectToday,
  onSelectSession,
  onOpenPalette,
  isMobile = false,
  inboxCollapsed = false,
  onToggleInbox = () => undefined,
}: AgentWatchNavProps) {
  const recentSessions = sessions
    .filter((session) => session.lastPromptedAt != null)
    .toSorted(
      (left, right) =>
        Date.parse(right.lastPromptedAt ?? '') -
        Date.parse(left.lastPromptedAt ?? ''),
    )
    .slice(0, 10)

  return (
    <nav className="agent-watch-nav" aria-label="Main navigation">
      <div className="brand-row">
        <span className="brand-mark" aria-hidden="true">
          &gt;
        </span>
        <span className="brand-name">agent watch</span>
        <button
          aria-label={
            isMobile
              ? 'Open command palette'
              : `Jump to with ${paletteShortcut.ariaLabel}`
          }
          className="agent-watch-command agent-watch-command-key"
          onClick={onOpenPalette}
          type="button"
        >
          <Search aria-hidden="true" size={13} />
          {!isMobile ? <span>{paletteShortcut.label}</span> : null}
        </button>
        {!isMobile ? (
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
              <PanelLeftOpen aria-hidden="true" size={15} />
            ) : (
              <PanelLeftClose aria-hidden="true" size={15} />
            )}
          </button>
        ) : null}
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
          <span>Inbox&nbsp;</span>
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
                  <Filter aria-hidden="true" size={12} />
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
        <h2 id="today-nav-title">Starred</h2>
        <ul>
          {todaySessions.map((session) => (
            <li key={session.id}>
              <Button
                type="button"
                variant="plain"
                className="session-row cursor-pointer"
                aria-label={`Open starred session ${session.title}`}
                onClick={() => {
                  onSelectToday(session.id)
                }}
              >
                <SessionRow session={session} density="compact" />
              </Button>
            </li>
          ))}
        </ul>
      </section>
      {recentSessions.length > 0 ? (
        <section className="recent-nav" aria-labelledby="recent-nav-title">
          <h2 id="recent-nav-title">Recent</h2>
          <ul>
            {recentSessions.map((session) => (
              <li key={session.id}>
                <Button
                  type="button"
                  variant="plain"
                  className="session-row cursor-pointer"
                  aria-label={`Open ${session.title} recent session`}
                  onClick={() => {
                    onSelectSession(session.id)
                  }}
                >
                  <SessionRow session={session} density="compact" />
                </Button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {workTypeNavGroups.map((group, index) => (
        <section
          key={group.label}
          className="work-type-nav"
          aria-labelledby={`work-type-nav-title-${String(index)}`}
        >
          <h2 id={`work-type-nav-title-${String(index)}`}>{group.label}</h2>
          <ul>
            {group.sessions.map((session) => (
              <li key={session.id}>
                <Button
                  type="button"
                  variant="plain"
                  className="session-row cursor-pointer"
                  aria-label={`Open ${session.title} ${group.label} session in Inbox`}
                  onClick={() => {
                    onSelectSession(session.id)
                  }}
                >
                  <SessionRow session={session} density="compact" />
                </Button>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </nav>
  )
}
