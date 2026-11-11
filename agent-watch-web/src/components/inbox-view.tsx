import '#components/later-today.css'

import { Button } from '@fohte/ui/button'
import { Filter, X } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'

import { InboxFilterBar } from '#components/inbox-filter-bar'
import { InboxFilterDialog } from '#components/inbox-filter-dialog'
import { SessionRow } from '#components/session-row'
import { filterInboxSessions, parseInboxFilterQuery } from '#inbox-filter'
import { nextReturnLabel } from '#inbox-view-utils'
import type { SavedInboxView } from '#saved-inbox-views'
import type { Session } from '#types'

type InboxViewProps = {
  sessions: readonly Session[]
  selectedId: string | null
  loaded: boolean
  error: string | null
  showAllInProgress: boolean
  onShowMore: () => void
  onSelect: (id: string) => void
  laterExpanded: boolean
  onToggleLater: () => void
  onLater: (id: string) => void
  onToggleToday: (id: string, isToday: boolean) => void
  todaySessions: readonly Session[]
  onSelectToday: (id: string) => void
  isMobile: boolean
  inProgressExpanded: boolean
  onToggleInProgress: () => void
  doneExpanded: boolean
  onToggleDone: () => void
  filterQuery?: string
  onFilterQueryChange?: (query: string) => void
  filterDialogOpen?: boolean
  onFilterDialogOpenChange?: (open: boolean) => void
  saveViewOpen?: boolean
  saveViewName?: string
  onSaveViewOpenChange?: (open: boolean) => void
  onSaveViewNameChange?: (name: string) => void
  onSaveView?: (name: string) => void
  savedViews?: readonly (SavedInboxView & {
    count: number
    countTone: 'needs-action' | 'error' | null
  })[]
  selectedSavedViewId?: string | null
  onSelectSavedView?: (id: string) => void
  onDeleteSavedView?: (id: string) => void
}

function renderSessionSection({
  title,
  sessions,
  count = sessions.length,
  selectedId,
  onSelect,
  onLater,
  onToggleToday,
  compactRepo,
  isMobile,
  className = '',
}: {
  title: string
  sessions: readonly Session[]
  count?: number
  selectedId: string | null
  onSelect: (id: string) => void
  onLater: (id: string) => void
  onToggleToday: (id: string, isToday: boolean) => void
  compactRepo: boolean
  isMobile: boolean
  className?: string
}) {
  return (
    <section className={`session-section ${className}`} aria-label={title}>
      <h2 className="section-heading">
        <span>{title}</span>
        <span className="section-count">{count}</span>
      </h2>
      {sessions.length > 0 ? (
        <ul className="session-list">
          {sessions.map((session) => (
            <SessionRow
              key={session.id}
              session={session}
              selected={session.id === selectedId}
              onSelect={onSelect}
              onLater={onLater}
              onToggleToday={onToggleToday}
              compactRepo={compactRepo}
              isMobile={isMobile}
            />
          ))}
        </ul>
      ) : null}
    </section>
  )
}

export function InboxView({
  sessions,
  selectedId,
  loaded,
  error,
  showAllInProgress,
  onShowMore,
  onSelect,
  laterExpanded,
  onToggleLater,
  onLater,
  onToggleToday,
  todaySessions,
  onSelectToday,
  isMobile,
  inProgressExpanded,
  onToggleInProgress,
  doneExpanded,
  onToggleDone,
  filterQuery: controlledFilterQuery,
  onFilterQueryChange,
  filterDialogOpen: controlledFilterDialogOpen,
  onFilterDialogOpenChange,
  saveViewOpen = false,
  saveViewName = '',
  onSaveViewOpenChange,
  onSaveViewNameChange,
  onSaveView,
  savedViews = [],
  selectedSavedViewId = null,
  onSelectSavedView,
  onDeleteSavedView,
}: InboxViewProps) {
  const [localFilterQuery, setLocalFilterQuery] = useState('')
  const [localDialogOpen, setLocalDialogOpen] = useState(false)
  const [draftQuery, setDraftQuery] = useState(controlledFilterQuery ?? '')
  const filterQuery = controlledFilterQuery ?? localFilterQuery
  const filterDialogOpen = controlledFilterDialogOpen ?? localDialogOpen
  const filters = parseInboxFilterQuery(filterQuery)
  const filteredSessions = filterInboxSessions(sessions, filters)
  const allNeedsActionCount = sessions.filter(
    (session) => session.category === 'needs_action',
  ).length
  const needsAction = filteredSessions.filter(
    (session) => session.category === 'needs_action',
  )
  const inProgress = filteredSessions.filter(
    (session) => session.category === 'in_progress',
  )
  const done = filteredSessions.filter((session) => session.category === 'done')
  const unreviewedDoneCount = done.reduce(
    (count, session) => count + (session.unreviewedDecisionCount ?? 0),
    0,
  )
  const later = filteredSessions.filter(
    (session) => session.category === 'later',
  )
  const setFilterQuery = useCallback(
    (query: string) => {
      if (onFilterQueryChange !== undefined) {
        onFilterQueryChange(query)
      } else {
        setLocalFilterQuery(query)
      }
    },
    [onFilterQueryChange],
  )
  const setFilterDialogOpen = useCallback(
    (open: boolean) => {
      if (onFilterDialogOpenChange !== undefined) {
        onFilterDialogOpenChange(open)
      } else {
        setLocalDialogOpen(open)
      }
    },
    [onFilterDialogOpenChange],
  )
  const openFilterDialog = useCallback(() => {
    setDraftQuery(filterQuery)
    setFilterDialogOpen(true)
  }, [filterQuery, setFilterDialogOpen])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== '/' || event.defaultPrevented) return
      const target = event.target
      if (
        target instanceof HTMLElement &&
        (target.isContentEditable ||
          target instanceof HTMLInputElement ||
          target instanceof HTMLTextAreaElement ||
          target instanceof HTMLSelectElement)
      ) {
        return
      }
      if (!filterDialogOpen) {
        event.preventDefault()
        openFilterDialog()
      }
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [filterDialogOpen, openFilterDialog])

  const visibleInProgress = isMobile
    ? inProgressExpanded
      ? inProgress
      : []
    : showAllInProgress
      ? inProgress
      : inProgress.slice(0, 2)
  const hiddenInProgressCount = inProgress.length - visibleInProgress.length

  return (
    <section className="inbox-view" aria-labelledby="inbox-title">
      <div className="mobile-inbox-brand">
        <span className="brand-mark" aria-hidden="true">
          &gt;
        </span>
        <span>agent watch</span>
        <span className="mobile-inbox-connection">
          <span className="mobile-inbox-connection-dot" aria-hidden="true" />
          Connected
        </span>
      </div>
      <header className="view-header">
        <h1 id="inbox-title">Inbox</h1>
        <span className="sort-description">Sorted by tq priority</span>
      </header>
      <InboxFilterBar
        filters={filters}
        matchingNeedsActionCount={needsAction.length}
        totalNeedsActionCount={allNeedsActionCount}
        onFilterQueryChange={setFilterQuery}
        onOpenFilterDialog={openFilterDialog}
        saveViewOpen={saveViewOpen}
        saveViewName={saveViewName}
        onSaveViewOpenChange={onSaveViewOpenChange}
        onSaveViewNameChange={onSaveViewNameChange}
        onSaveView={onSaveView}
      />
      {isMobile && savedViews.length > 0 ? (
        <section
          className="mobile-saved-views"
          aria-labelledby="mobile-saved-views-title"
        >
          <h2 id="mobile-saved-views-title">VIEWS</h2>
          <ul>
            {savedViews.map((savedView) => (
              <li key={savedView.id}>
                <Button
                  type="button"
                  variant="plain"
                  aria-pressed={selectedSavedViewId === savedView.id}
                  className={`mobile-saved-view-link${selectedSavedViewId === savedView.id ? ' is-active' : ''}`}
                  onClick={() => {
                    onSelectSavedView?.(savedView.id)
                  }}
                >
                  <Filter aria-hidden="true" size={13} />
                  <span className="mobile-saved-view-name">
                    {savedView.name}
                  </span>
                  <span
                    className={`mobile-saved-view-count${savedView.countTone === null ? '' : ` is-${savedView.countTone}`}`}
                  >
                    {savedView.count}
                  </span>
                </Button>
                <Button
                  type="button"
                  variant="plain"
                  aria-label={`Delete ${savedView.name} view`}
                  className="mobile-saved-view-delete"
                  onClick={() => {
                    onDeleteSavedView?.(savedView.id)
                  }}
                >
                  <X aria-hidden="true" size={13} />
                </Button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      <div className="mobile-today-strip" aria-label="TODAY sessions">
        {filterInboxSessions(todaySessions, filters).map((session) => (
          <Button
            key={session.id}
            type="button"
            variant="plain"
            className={`mobile-today-chip cursor-pointer${session.category === 'needs_action' ? ' is-needs-action' : ''}`}
            onClick={() => {
              onSelectToday(session.id)
            }}
          >
            <span aria-hidden="true" className="today-star">
              ★
            </span>
            <span>{session.title}</span>
          </Button>
        ))}
      </div>
      <div className="inbox-scroll-area">
        {error !== null ? (
          <p className="load-message is-error" role="status">
            {error}
          </p>
        ) : null}
        {!loaded && error === null ? (
          <p className="load-message" role="status">
            Loading sessions…
          </p>
        ) : null}
        {loaded && sessions.length === 0 ? (
          <p className="load-message">No sessions to show.</p>
        ) : null}
        {loaded && sessions.length > 0 && filteredSessions.length === 0 ? (
          <p className="load-message">No sessions match these filters.</p>
        ) : null}
        {renderSessionSection({
          title: 'Needs action',
          className: 'needs-action-section',
          sessions: needsAction,
          selectedId,
          onSelect,
          onLater,
          onToggleToday,
          compactRepo: isMobile,
          isMobile,
        })}
        <Button
          type="button"
          variant="plain"
          className="later-collapsed-row cursor-pointer"
          aria-expanded={laterExpanded}
          onClick={onToggleLater}
        >
          <span>{isMobile ? 'Later' : '◷ Later'}</span>
          <span className="later-section-count">{later.length}</span>
          {later.length > 0 || isMobile ? (
            <span className="later-next-return">
              {later.length > 0 ? nextReturnLabel(later) : 'Next activity'}{' '}
              {isMobile
                ? laterExpanded
                  ? '⌄'
                  : '›'
                : laterExpanded
                  ? '▾'
                  : '▸'}
            </span>
          ) : null}
        </Button>
        {laterExpanded && later.length > 0 ? (
          <ul className="session-list later-session-list">
            {later.map((session) => (
              <SessionRow
                key={session.id}
                session={session}
                selected={session.id === selectedId}
                onSelect={onSelect}
                onLater={onLater}
                onToggleToday={onToggleToday}
                compactRepo={isMobile}
                isMobile={isMobile}
              />
            ))}
          </ul>
        ) : null}
        <Button
          type="button"
          variant="plain"
          className="mobile-collapse-row mobile-in-progress-row cursor-pointer"
          aria-expanded={inProgressExpanded}
          onClick={onToggleInProgress}
        >
          <span>In progress</span>
          <span className="mobile-collapse-count">{inProgress.length}</span>
          <span className="mobile-collapse-chevron" aria-hidden="true">
            {inProgressExpanded ? '⌄' : '›'}
          </span>
        </Button>
        {renderSessionSection({
          title: 'In progress',
          className: 'in-progress-section',
          sessions: visibleInProgress,
          count: inProgress.length,
          selectedId,
          onSelect,
          onLater,
          onToggleToday,
          compactRepo: isMobile,
          isMobile,
        })}
        {hiddenInProgressCount > 0 && !isMobile ? (
          <Button
            className="show-more-button cursor-pointer"
            type="button"
            variant="plain"
            aria-expanded={showAllInProgress}
            onClick={onShowMore}
          >
            {hiddenInProgressCount} more ▸
          </Button>
        ) : null}
        <Button
          type="button"
          variant="plain"
          className="mobile-collapse-row mobile-done-row cursor-pointer"
          aria-expanded={doneExpanded}
          onClick={onToggleDone}
        >
          <span>Done</span>
          <span className="mobile-collapse-count">{done.length}</span>
          {unreviewedDoneCount > 0 ? (
            <span className="mobile-done-review-count">
              ◇ {String(unreviewedDoneCount)} to review ›
            </span>
          ) : null}
          {unreviewedDoneCount === 0 ? (
            <span className="mobile-collapse-chevron" aria-hidden="true">
              {doneExpanded ? '⌄' : '›'}
            </span>
          ) : null}
        </Button>
        {renderSessionSection({
          title: 'Done',
          className: 'done-section',
          sessions: isMobile && !doneExpanded ? [] : done,
          selectedId,
          onSelect,
          onLater,
          onToggleToday,
          compactRepo: isMobile,
          isMobile,
        })}
      </div>
      <footer className="inbox-footer" aria-label="Keyboard shortcuts">
        <span>
          <kbd>j</kbd> <kbd>k</kbd> Move
        </span>
        <span>
          <kbd>L</kbd> Later
        </span>
        <span>
          <kbd>⇧B</kbd> Board
        </span>
        <span>
          <kbd>[</kbd> Inbox
        </span>
        <span>
          <kbd>⌘.</kbd> Panel
        </span>
      </footer>
      {filterDialogOpen ? (
        <InboxFilterDialog
          sessions={sessions}
          query={draftQuery}
          onQueryChange={setDraftQuery}
          onApply={() => {
            setFilterQuery(draftQuery)
            setFilterDialogOpen(false)
          }}
          onClose={() => {
            setFilterDialogOpen(false)
          }}
        />
      ) : null}
    </section>
  )
}
