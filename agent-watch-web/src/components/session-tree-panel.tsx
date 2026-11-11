import '#components/session-tree-panel.css'

import type { KeyboardEvent } from 'react'
import { useEffect, useMemo, useRef, useState } from 'react'

import { formatElapsedTime } from '#components/format-elapsed-time'
import {
  defaultExpandedSessionIds,
  visibleSessionTreeRows,
} from '#session-tree-model'
import type { SessionTreeNode } from '#types'

type SessionTreePanelProps = {
  sessions: readonly SessionTreeNode[]
  selectedSessionId: string
  onSelectSession: (id: string) => void
}

export function SessionTreePanel({
  sessions,
  selectedSessionId,
  onSelectSession,
}: SessionTreePanelProps) {
  const treeRef = useRef<HTMLDivElement>(null)
  const [focusedId, setFocusedId] = useState(selectedSessionId)
  const [treeReceivedAt, setTreeReceivedAt] = useState(() => Date.now())
  const [nowMs, setNowMs] = useState(() => Date.now())
  const [expandedIds, setExpandedIds] = useState<Set<string>>(() =>
    defaultExpandedSessionIds(sessions, selectedSessionId),
  )
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
    const interval = window.setInterval(() => {
      setNowMs(Date.now())
    }, 1_000)
    return () => {
      window.clearInterval(interval)
    }
  }, [])

  const updateFocus = (id: string) => {
    setFocusedId(id)
    treeRef.current?.focus()
  }
  const toggleExpanded = (id: string) => {
    setExpandedIds((current) => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
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
      if (next !== undefined) setFocusedId(next.session.id)
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
        if (parent !== undefined) setFocusedId(parent.session.id)
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
        if (child !== undefined) setFocusedId(child.session.id)
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
    <div className="session-tree-panel">
      <div
        ref={treeRef}
        className="session-tree-list"
        role="tree"
        tabIndex={0}
        aria-label={`Session tree for ${sessionById.get(selectedSessionId)?.title ?? 'current session'}`}
        aria-activedescendant={`session-tree-row-${focusedId}`}
        onKeyDown={handleKeyDown}
      >
        {rows.map((row) => {
          const { session } = row
          const parent =
            session.parentId === null
              ? undefined
              : sessionById.get(session.parentId)
          const taskDiffers =
            session.task !== null && session.task.id !== parent?.task?.id
          const repoDiffers =
            session.repo !== '' && session.repo !== parent?.repo
          const kind = session.kind ?? 'Session'
          const directChildCount = sessions.filter(
            (child) => child.parentId === session.id,
          ).length
          const elapsedSeconds =
            session.elapsedSeconds +
            Math.max(0, Math.floor((nowMs - treeReceivedAt) / 1_000))
          const preview =
            session.category === 'done' &&
            session.pullRequest?.state === 'Merged'
              ? `Merged #${String(session.pullRequest.number)} · ◇ ${String(session.decisionCount)} decisions · ${String(directChildCount)} ${directChildCount === 1 ? 'child' : 'children'}`
              : session.preview
          return (
            <div
              key={session.id}
              id={`session-tree-row-${session.id}`}
              className={`session-tree-row${session.id === selectedSessionId ? ' is-current' : ''}${session.id === focusedId ? ' is-focused' : ''}${session.isActive ? '' : ' is-ended'}`}
              role="treeitem"
              aria-level={row.depth + 1}
              aria-expanded={row.hasChildren ? row.isExpanded : undefined}
              aria-selected={session.id === selectedSessionId}
              style={{ paddingLeft: `${String(row.depth * 18 + 12)}px` }}
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
                <span
                  className={`session-tree-status session-tree-status--${session.category}`}
                  aria-label={session.category.replace('_', ' ')}
                />
                <span className="session-tree-title">{session.title}</span>
                {row.needsActionDescendantCount > 0 &&
                row.hasChildren &&
                !row.isExpanded ? (
                  <span className="session-tree-descendant-count">
                    {row.needsActionDescendantCount}
                  </span>
                ) : null}
                <span className="session-tree-age">
                  {elapsedSeconds === 0
                    ? 'now'
                    : formatElapsedTime(elapsedSeconds)}
                </span>
              </div>
              <div className="session-tree-detail-line">
                <span
                  className={`session-tree-kind session-tree-kind--${kind.toLowerCase()}`}
                >
                  {kind}
                </span>
                {preview === null || preview === '' ? null : (
                  <span className="session-tree-preview"> · {preview}</span>
                )}
              </div>
              {taskDiffers || repoDiffers ? (
                <div className="session-tree-meta-line">
                  {taskDiffers ? (
                    <span className="session-tree-task">
                      <span className="session-tree-task-id">
                        {String(session.task?.number)}
                      </span>
                      <span> · {session.task?.title}</span>
                    </span>
                  ) : null}
                  {repoDiffers ? (
                    <span className="session-tree-repo">{session.repo}</span>
                  ) : null}
                </div>
              ) : null}
            </div>
          )
        })}
      </div>
      <footer className="session-tree-footer">
        tq and repo are shown only when they differ from the parent. ← / → fold.
      </footer>
    </div>
  )
}
