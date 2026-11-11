import '#components/command-palette.css'

import { useEffect, useMemo, useRef, useState } from 'react'

import {
  buildCommandPaletteGroups,
  type PaletteItem,
  type PaletteScope,
} from '#command-palette-model'
import {
  itemColor,
  itemDetail,
  itemIcon,
  itemResultTitle,
  itemSubtitle,
  itemTitle,
  sessionKindLabel,
} from '#components/command-palette-item'
import { useCopySessionId } from '#components/use-copy-session-id'
import type { Session } from '#types'

type CommandPaletteProps = {
  sessions: readonly Session[]
  recentSessionIds: readonly string[]
  initialQuery?: string
  initialScope?: PaletteScope
  initialActionItemId?: string
  onClose: () => void
  onOpenSession: (id: string) => void
  onLater: (id: string) => void
  onOpenInTerminal: (id: string) => void
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

export function CommandPalette({
  sessions,
  recentSessionIds,
  initialQuery = '',
  initialScope = 'all',
  initialActionItemId,
  onClose,
  onOpenSession,
  onLater,
  onOpenInTerminal,
  onOpenProject,
  onRunCommand,
}: CommandPaletteProps) {
  const [query, setQuery] = useState(initialQuery)
  const [scope, setScope] = useState<PaletteScope>(initialScope)
  const [activeIndex, setActiveIndex] = useState(0)
  const [actionsFor, setActionsFor] = useState<PaletteItem | null>(
    () =>
      sessions
        .filter((session) => session.id === initialActionItemId)
        .map((session) => ({
          type: 'session' as const,
          id: session.id,
          session,
        }))[0] ?? null,
  )
  const [activeActionIndex, setActiveActionIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const groups = useMemo(
    () => buildCommandPaletteGroups(sessions, query, scope, recentSessionIds),
    [query, recentSessionIds, scope, sessions],
  )
  const items = groups.flatMap((group) => group.items)
  const actions =
    actionsFor?.type === 'session'
      ? [
          { id: 'open', label: 'Open session' },
          { id: 'later', label: 'Later' },
          { id: 'copy', label: 'Copy session ID' },
          ...(actionsFor.session.canOpenInTerminal === true
            ? [{ id: 'terminal', label: 'Open in terminal' }]
            : []),
        ]
      : actionsFor?.type === 'command'
        ? [{ id: 'command', label: 'Run command' }]
        : actionsFor === null
          ? []
          : [{ id: 'destination', label: 'Show in Board' }]
  const action = actions[activeActionIndex]
  const { status: copyStatus, copySessionId } = useCopySessionId(
    actionsFor?.type === 'session' ? actionsFor.session.id : '',
  )

  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  useEffect(() => {
    if (actionsFor === null) {
      inputRef.current?.focus()
      return
    }
    document
      .querySelector<HTMLButtonElement>(
        '[data-palette-action-active="true"] button',
      )
      ?.focus()
  }, [actionsFor, activeActionIndex])

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

  function activateAction(action: (typeof actions)[number] | undefined) {
    if (action === undefined || actionsFor === null) return
    switch (action.id) {
      case 'open':
        if (actionsFor.type === 'session') onOpenSession(actionsFor.session.id)
        setActionsFor(null)
        return
      case 'later':
        if (actionsFor.type === 'session') onLater(actionsFor.session.id)
        setActionsFor(null)
        return
      case 'copy':
        copySessionId()
        return
      case 'terminal':
        if (actionsFor.type === 'session') {
          onOpenInTerminal(actionsFor.session.id)
        }
        setActionsFor(null)
        return
      case 'destination':
        activate(actionsFor)
        setActionsFor(null)
        return
      case 'command':
        if (actionsFor.type === 'command') onRunCommand(actionsFor)
        setActionsFor(null)
    }
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
          if (actionsFor !== null) {
            if (event.key === 'Escape' || event.key === 'ArrowLeft') {
              event.preventDefault()
              setActionsFor(null)
              return
            }
            if (event.key === 'ArrowDown') {
              event.preventDefault()
              setActiveActionIndex((index) =>
                Math.min(actions.length - 1, index + 1),
              )
              return
            }
            if (event.key === 'ArrowUp') {
              event.preventDefault()
              setActiveActionIndex((index) => Math.max(0, index - 1))
              return
            }
            if (event.key === 'Enter') {
              event.preventDefault()
              activateAction(action)
              return
            }
          }
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
          } else if (event.key === 'ArrowRight') {
            const input = inputRef.current
            if (
              event.target === input &&
              (input.selectionStart !== input.value.length ||
                input.selectionStart !== input.selectionEnd)
            ) {
              return
            }
            event.preventDefault()
            const item = items[activeIndex]
            if (item !== undefined) {
              setActionsFor(item)
              setActiveActionIndex(0)
            }
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
          {actionsFor !== null ? (
            <section
              aria-label={`${itemTitle(actionsFor)} actions`}
              className="command-palette-group"
              role="menu"
            >
              <h2>{itemTitle(actionsFor)}</h2>
              {actions.map((action, index) => (
                <div
                  key={action.id}
                  aria-current={
                    index === activeActionIndex ? 'true' : undefined
                  }
                  className={`command-palette-action${index === activeActionIndex ? ' is-active' : ''}`}
                  data-palette-action-active={
                    index === activeActionIndex ? 'true' : 'false'
                  }
                  role="none"
                  onMouseEnter={() => {
                    setActiveActionIndex(index)
                  }}
                >
                  <button
                    role="menuitem"
                    type="button"
                    onClick={() => {
                      activateAction(action)
                    }}
                  >
                    {action.id === 'copy'
                      ? copyStatus === 'copied'
                        ? 'Copied'
                        : copyStatus === 'failed'
                          ? 'Copy failed'
                          : 'Copy session ID'
                      : action.label}
                  </button>
                </div>
              ))}
            </section>
          ) : groups.length === 0 ? (
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
                      >
                        {itemIcon(item)}
                      </span>
                      <span className="command-palette-item-main">
                        <span className="command-palette-item-heading">
                          <strong>{itemResultTitle(item)}</strong>
                          {item.type === 'session' ? (
                            <span className="command-palette-item-kind">
                              {sessionKindLabel(item.session)}
                            </span>
                          ) : null}
                        </span>
                        <span className="command-palette-item-subtitle">
                          {itemSubtitle(item)}
                        </span>
                      </span>
                      <span className="command-palette-item-detail is-monospace">
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
          {actionsFor !== null ? (
            <>
              <span>
                <kbd>↵</kbd> Select
              </span>
              <span>
                <kbd>←</kbd> Back
              </span>
              <span>
                <kbd>↑↓</kbd> Navigate
              </span>
            </>
          ) : (
            <>
              <span>
                <kbd>↵</kbd> Open
              </span>
              <span>
                <kbd>→</kbd> Actions
              </span>
              <span>
                <kbd>Tab</kbd> Scope
              </span>
              <span className="command-palette-hint-legend">
                <kbd>#</kbd> task · <kbd>@</kbd> project · <kbd>&gt;</kbd>{' '}
                command
              </span>
            </>
          )}
        </footer>
      </section>
    </div>
  )
}
