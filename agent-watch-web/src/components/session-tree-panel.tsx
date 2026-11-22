import '#components/session-tree-panel.css'

import type { KeyboardEvent } from 'react'
import { useEffect, useMemo, useRef, useState } from 'react'

import { SessionRow } from '#components/session-row'
import {
  defaultExpandedSessionIds,
  type SessionTreeViewState,
  visibleSessionTreeRows,
} from '#session-tree-model'
import type { SessionTreeNode } from '#types'

type SessionTreePanelProps = {
  sessions: readonly SessionTreeNode[]
  selectedSessionId: string
  onSelectSession: (id: string) => void
  size?: 'desktop' | 'phone'
  viewState?: SessionTreeViewState | null
  onViewStateChange?: (state: SessionTreeViewState) => void
}

export function SessionTreePanel({
  sessions,
  selectedSessionId,
  onSelectSession,
  size = 'desktop',
  viewState: controlledViewState,
  onViewStateChange,
}: SessionTreePanelProps) {
  const treeRef = useRef<HTMLDivElement>(null)
  const [treeReceivedAt, setTreeReceivedAt] = useState(() => Date.now())
  const [nowMs, setNowMs] = useState(() => Date.now())
  const [localViewState, setLocalViewState] = useState<SessionTreeViewState>(
    () => ({
      expandedSessionIds: [
        ...defaultExpandedSessionIds(sessions, selectedSessionId),
      ],
      focusedSessionId: selectedSessionId,
      treeHasFocus: false,
    }),
  )
  const viewState = controlledViewState ?? localViewState
  const viewStateRef = useRef(viewState)
  viewStateRef.current = viewState
  const expandedIds = useMemo(
    () => new Set(viewState.expandedSessionIds),
    [viewState.expandedSessionIds],
  )
  const focusedId = viewState.focusedSessionId
  const rows = useMemo(
    () => visibleSessionTreeRows(sessions, expandedIds),
    [expandedIds, sessions],
  )
  const sessionById = useMemo(
    () => new Map(sessions.map((session) => [session.id, session])),
    [sessions],
  )
  useEffect(() => {
    setTreeReceivedAt(Date.now())
  }, [sessions])

  useEffect(() => {
    if (viewState.treeHasFocus) treeRef.current?.focus()
  }, [viewState.treeHasFocus])

  useEffect(() => {
    const interval = window.setInterval(() => {
      setNowMs(Date.now())
    }, 1_000)
    return () => {
      window.clearInterval(interval)
    }
  }, [])

  const updateViewState = (
    update: (current: SessionTreeViewState) => SessionTreeViewState,
  ) => {
    const current = viewStateRef.current
    const next = update(current)
    if (next === current) return
    viewStateRef.current = next
    if (controlledViewState == null) setLocalViewState(next)
    onViewStateChange?.(next)
  }
  const updateFocus = (id: string) => {
    updateViewState((current) => ({
      ...current,
      focusedSessionId: id,
      treeHasFocus: true,
    }))
    treeRef.current?.focus()
  }
  const updateTreeFocus = (treeHasFocus: boolean) => {
    updateViewState((current) =>
      current.treeHasFocus === treeHasFocus
        ? current
        : { ...current, treeHasFocus },
    )
  }
  const toggleExpanded = (id: string) => {
    updateViewState((current) => {
      const expanded = new Set(current.expandedSessionIds)
      if (expanded.has(id)) expanded.delete(id)
      else expanded.add(id)
      return { ...current, expandedSessionIds: [...expanded] }
    })
  }
  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== treeRef.current) return
    const currentIndex = rows.findIndex((row) => row.session.id === focusedId)
    const currentRow = rows[currentIndex]
    if (event.key === 'j' || event.key === 'k') {
      event.preventDefault()
      const nextIndex = Math.min(
        rows.length - 1,
        Math.max(0, currentIndex + (event.key === 'j' ? 1 : -1)),
      )
      const next = rows[nextIndex]
      if (next !== undefined) updateFocus(next.session.id)
      return
    }
    if (currentRow === undefined) return
    if (event.key === 'ArrowLeft') {
      event.preventDefault()
      if (currentRow.hasChildren && currentRow.isExpanded) {
        toggleExpanded(currentRow.session.id)
      } else if (currentRow.session.parentId !== null) {
        const parent = rows.find(
          (row) => row.session.id === currentRow.session.parentId,
        )
        if (parent !== undefined) updateFocus(parent.session.id)
      }
      return
    }
    if (event.key === 'ArrowRight') {
      event.preventDefault()
      if (currentRow.hasChildren && !currentRow.isExpanded) {
        toggleExpanded(currentRow.session.id)
      } else {
        const child = rows.find(
          (row) => row.session.parentId === currentRow.session.id,
        )
        if (child !== undefined) updateFocus(child.session.id)
      }
      return
    }
    if (event.key === 'Enter') {
      event.preventDefault()
      if (currentRow.session.isActive) {
        onSelectSession(currentRow.session.id)
      }
    }
  }

  return (
    <div className="session-tree-panel" data-size={size}>
      <div
        ref={treeRef}
        className="session-tree-list"
        role="tree"
        tabIndex={0}
        aria-label={`Session tree for ${sessionById.get(selectedSessionId)?.title ?? 'current session'}`}
        aria-activedescendant={`session-tree-row-${focusedId}`}
        onFocus={() => {
          updateTreeFocus(true)
        }}
        onBlur={(event) => {
          const nextTarget = event.relatedTarget
          if (
            nextTarget !== null &&
            !event.currentTarget.contains(nextTarget)
          ) {
            updateTreeFocus(false)
          }
        }}
        onKeyDown={handleKeyDown}
      >
        {rows.map((row) => {
          const { session } = row
          const elapsedSeconds =
            session.elapsedSeconds +
            Math.max(0, Math.floor((nowMs - treeReceivedAt) / 1_000))
          return (
            <div
              key={session.id}
              id={`session-tree-row-${session.id}`}
              className={`session-tree-row${session.id === selectedSessionId ? ' is-current' : ''}${session.id === focusedId ? ' is-focused' : ''}${session.isActive ? '' : ' is-ended'}`}
              role="treeitem"
              aria-level={row.depth + 1}
              aria-expanded={row.hasChildren ? row.isExpanded : undefined}
              aria-selected={session.id === selectedSessionId}
              style={{
                paddingLeft: `${String(row.depth * 18 + 12)}px`,
              }}
              onClick={() => {
                updateFocus(session.id)
                if (session.isActive) onSelectSession(session.id)
              }}
            >
              <div className="session-tree-main-line">
                {row.hasChildren ? (
                  <button
                    type="button"
                    className="session-tree-caret"
                    aria-label={`${row.isExpanded ? 'Collapse' : 'Expand'} ${session.title}`}
                    data-tree-caret=""
                    onClick={(event) => {
                      event.stopPropagation()
                      updateFocus(session.id)
                      toggleExpanded(session.id)
                    }}
                  >
                    {row.isExpanded ? '▾' : '▸'}
                  </button>
                ) : (
                  <span
                    className="session-tree-caret-space"
                    aria-hidden="true"
                  />
                )}
                <SessionRow
                  session={{ ...session, elapsedSeconds }}
                  density={size === 'phone' ? 'compact-phone' : 'compact'}
                />
              </div>
            </div>
          )
        })}
      </div>
      <footer className="session-tree-footer">← / → fold.</footer>
    </div>
  )
}
