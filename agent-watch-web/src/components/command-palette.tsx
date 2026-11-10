import '#components/command-palette.css'

import { useEffect, useMemo, useRef, useState } from 'react'

import {
  buildCommandPaletteGroups,
  type PaletteItem,
  type PaletteScope,
} from '#command-palette-model'
import type { Session } from '#types'

type CommandPaletteProps = {
  sessions: readonly Session[]
  recentSessionIds: readonly string[]
  inboxFilterQuery: string
  initialQuery?: string
  initialScope?: PaletteScope
  onClose: () => void
  onOpenSession: (id: string) => void
  onOpenProject: (project: string) => void
  onRunCommand: (command: Extract<PaletteItem, { type: 'command' }>) => void
}

const SCOPES: Array<{ key: PaletteScope; label: string; marker?: string }> = [
  { key: 'all', label: 'All' },
  { key: 'sessions', label: 'Sessions' },
  { key: 'tasks', label: 'tq tasks', marker: '#' },
  { key: 'projects', label: 'Projects', marker: '@' },
  { key: 'commands', label: 'Commands', marker: '>' },
]

function itemTitle(item: PaletteItem) {
  switch (item.type) {
    case 'session':
      return item.session.title
    case 'task':
      return item.task.title
    case 'project':
      return item.name
    case 'command':
      return item.label
  }
}

function itemSubtitle(item: PaletteItem) {
  switch (item.type) {
    case 'session': {
      const task =
        item.session.task === null
          ? ''
          : `P${String(item.session.task.priority ?? '—')} #${String(item.session.task.number)} ${item.session.task.title} · `
      return `${task}${item.session.project} · ${item.session.repo}`
    }
    case 'task':
      return aggregateSubtitle(item.sessions)
    case 'project':
      return aggregateSubtitle(item.sessions)
    case 'command':
      return item.description
  }
}

function aggregateSubtitle(sessions: readonly Session[]) {
  const needsActionCount = sessions.filter(
    (session) => session.category === 'needs_action',
  ).length
  const repos = Array.from(new Set(sessions.map((session) => session.repo)))
  return `${String(sessions.length)} sessions · ${String(needsActionCount)} need action · ${repos.join(', ')}`
}

function itemDetail(item: PaletteItem) {
  switch (item.type) {
    case 'session':
      return item.session.category === 'needs_action'
        ? `${item.session.kind ?? 'Needs action'} · ${String(Math.floor(item.session.elapsedSeconds / 60))}m`
        : `${item.session.category.replace('_', ' ')} · ${String(Math.floor(item.session.elapsedSeconds / 60))}m`
    case 'task':
      return 'Show in Board'
    case 'project':
      return 'Show in Board'
    case 'command':
      return '↵'
  }
}

function itemColor(item: PaletteItem) {
  if (item.type === 'session') {
    return item.session.category.replace('_', '-')
  }
  if (item.type === 'command') return 'command'
  return item.type
}

export function CommandPalette({
  sessions,
  recentSessionIds,
  inboxFilterQuery,
  initialQuery = '',
  initialScope = 'all',
  onClose,
  onOpenSession,
  onOpenProject,
  onRunCommand,
}: CommandPaletteProps) {
  const [query, setQuery] = useState(initialQuery)
  const [scope, setScope] = useState<PaletteScope>(initialScope)
  const [activeIndex, setActiveIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const groups = useMemo(
    () =>
      buildCommandPaletteGroups(
        sessions,
        query,
        scope,
        recentSessionIds,
        inboxFilterQuery.length > 0,
      ),
    [inboxFilterQuery.length, query, recentSessionIds, scope, sessions],
  )
  const items = groups.flatMap((group) => group.items)

  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  useEffect(() => {
    setActiveIndex((index) => Math.min(index, Math.max(0, items.length - 1)))
  }, [items.length])

  useEffect(() => {
    document
      .querySelector<HTMLElement>('[data-palette-active="true"]')
      ?.scrollIntoView({ block: 'nearest' })
  }, [activeIndex])

  function activate(item: PaletteItem | undefined) {
    if (item === undefined) return
    if (item.type === 'session') {
      onOpenSession(item.session.id)
      return
    }
    if (item.type === 'project') {
      onOpenProject(item.name)
      return
    }
    if (item.type === 'task') {
      const firstProject = item.sessions[0]?.project
      const project =
        firstProject === undefined || firstProject.trim().length === 0
          ? 'Unsorted'
          : firstProject.trim()
      onOpenProject(project)
      return
    }
    onRunCommand(item)
  }

  function selectScope(nextScope: PaletteScope) {
    setScope(nextScope)
    setQuery((current) => current.trimStart().replace(/^[#@>]\s*/, ''))
    setActiveIndex(0)
  }

  return (
    <div className="command-palette-overlay" onMouseDown={onClose}>
      <section
        aria-label="Jump to"
        aria-modal="true"
        className="command-palette"
        role="dialog"
        onMouseDown={(event) => {
          event.stopPropagation()
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault()
            onClose()
          } else if (
            event.key === 'ArrowDown' ||
            (event.ctrlKey && event.key === 'n')
          ) {
            event.preventDefault()
            setActiveIndex((index) => Math.min(items.length - 1, index + 1))
          } else if (
            event.key === 'ArrowUp' ||
            (event.ctrlKey && event.key === 'p')
          ) {
            event.preventDefault()
            setActiveIndex((index) => Math.max(0, index - 1))
          } else if (event.key === 'Enter') {
            event.preventDefault()
            activate(items[activeIndex])
          } else if (event.key === 'Tab') {
            event.preventDefault()
            const currentIndex = SCOPES.findIndex((item) => item.key === scope)
            const offset = event.shiftKey ? -1 : 1
            const nextIndex =
              (currentIndex + offset + SCOPES.length) % SCOPES.length
            selectScope(SCOPES[nextIndex]?.key ?? 'all')
          }
        }}
      >
        <div className="command-palette-search">
          <span className="command-palette-search-icon" aria-hidden="true">
            ⌕
          </span>
          <input
            ref={inputRef}
            aria-label="Jump to a session, tq task or project, or type > for commands"
            autoComplete="off"
            onChange={(event) => {
              const nextQuery = event.currentTarget.value
              setQuery(nextQuery)
              const prefix = nextQuery.trimStart()[0]
              if (prefix === '#') setScope('tasks')
              if (prefix === '@') setScope('projects')
              if (prefix === '>') setScope('commands')
              setActiveIndex(0)
            }}
            placeholder="Jump to a session, tq task or project, or type > for commands"
            value={query}
          />
          <button
            className="command-palette-escape"
            onClick={onClose}
            type="button"
          >
            esc
          </button>
        </div>
        <div
          aria-label="Jump scope"
          className="command-palette-scopes"
          role="tablist"
        >
          {SCOPES.map((item) => (
            <button
              key={item.key}
              aria-selected={scope === item.key}
              className={scope === item.key ? 'is-active' : ''}
              onClick={() => {
                selectScope(item.key)
              }}
              role="tab"
              type="button"
            >
              {item.label}
              {item.marker !== undefined ? <kbd>{item.marker}</kbd> : null}
            </button>
          ))}
        </div>
        <div
          aria-label="Jump results"
          className="command-palette-results"
          role="listbox"
        >
          {groups.length === 0 ? (
            <p className="command-palette-empty">No results found</p>
          ) : (
            groups.map((group) => (
              <section className="command-palette-group" key={group.heading}>
                <h2>{group.heading}</h2>
                {group.items.map((item) => {
                  const index = items.indexOf(item)
                  const active = index === activeIndex
                  return (
                    <button
                      key={item.id}
                      aria-selected={active}
                      className="command-palette-item"
                      data-palette-active={active ? 'true' : 'false'}
                      data-tone={itemColor(item)}
                      onClick={() => {
                        activate(item)
                      }}
                      onMouseEnter={() => {
                        setActiveIndex(index)
                      }}
                      role="option"
                      type="button"
                    >
                      <span
                        className="command-palette-status"
                        aria-hidden="true"
                      />
                      <span className="command-palette-item-main">
                        <span className="command-palette-item-heading">
                          <strong>{itemTitle(item)}</strong>
                          {item.type === 'session' ? (
                            <span className="command-palette-item-kind">
                              {item.session.kind ??
                                item.session.category.replace('_', ' ')}
                            </span>
                          ) : item.type === 'task' ? (
                            <span className="command-palette-item-kind">
                              tq task
                            </span>
                          ) : item.type === 'project' ? (
                            <span className="command-palette-item-kind">
                              Project
                            </span>
                          ) : null}
                        </span>
                        <span className="command-palette-item-subtitle">
                          {itemSubtitle(item)}
                        </span>
                      </span>
                      <span className="command-palette-item-detail">
                        {itemDetail(item)}
                      </span>
                    </button>
                  )
                })}
              </section>
            ))
          )}
        </div>
        <footer className="command-palette-hints">
          <span>
            <kbd>↵</kbd> Open
          </span>
          <span>
            <kbd>Tab</kbd> Scope
          </span>
          <span>
            <kbd>#</kbd> task · <kbd>@</kbd> project · <kbd>&gt;</kbd> command
          </span>
        </footer>
      </section>
    </div>
  )
}
