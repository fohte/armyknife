import '#components/new-session.css'

import { Folder, GitBranch, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

import { SessionDirectoryOptions } from '#components/session-directory-options'
import type { NewSessionState } from '#use-new-session'

type NewSessionPickerProps = {
  state: NewSessionState
  isMobile?: boolean
  launchPending?: boolean
  onClose: () => void
  onLaunched: (launchId: string) => void
}

export function NewSessionPicker({
  state,
  isMobile = false,
  launchPending = false,
  onClose,
  onLaunched,
}: NewSessionPickerProps) {
  const [activeIndex, setActiveIndex] = useState(0)
  const firstMessageRef = useRef<HTMLInputElement>(null)
  const selectedDirectory = state.selectedDirectory

  useEffect(() => {
    if (selectedDirectory !== null) firstMessageRef.current?.focus()
  }, [selectedDirectory])

  return (
    <div className="new-session-picker-overlay" onMouseDown={onClose}>
      <section
        aria-label="New session"
        aria-modal="true"
        className={`new-session-picker${isMobile ? ' is-mobile' : ''}`}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault()
            onClose()
            return
          }
          if (event.altKey && event.key.toLowerCase() === 'w') {
            event.preventDefault()
            state.setWorktree(!state.worktree)
            return
          }
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault()
            if (selectedDirectory !== null) {
              state.setEngine(state.engine === 'claude' ? 'codex' : 'claude')
            } else {
              setActiveIndex((index) =>
                Math.max(
                  0,
                  Math.min(
                    state.directories.length - 1,
                    index + (event.key === 'ArrowDown' ? 1 : -1),
                  ),
                ),
              )
            }
            return
          }
          if (event.key === 'Enter') {
            event.preventDefault()
            if (selectedDirectory === null) {
              const directory = state.directories[activeIndex]
              if (directory !== undefined) state.chooseDirectory(directory)
              return
            }
            if (launchPending) return
            void state.start(onLaunched, event.altKey ? 'codex' : state.engine)
            return
          }
          if (
            event.key === 'Backspace' &&
            selectedDirectory !== null &&
            (event.target instanceof HTMLInputElement
              ? event.target.value.length === 0
              : true)
          ) {
            event.preventDefault()
            state.chooseDirectory(null)
          }
        }}
        onMouseDown={(event) => {
          event.stopPropagation()
        }}
        role="dialog"
      >
        <header className="new-session-picker-header">
          <span className="new-session-picker-crumb">› New session</span>
          <button
            aria-label={
              selectedDirectory === null
                ? 'Choose a directory'
                : `Change directory, currently ${selectedDirectory.path}`
            }
            aria-expanded={selectedDirectory === null}
            className="new-session-picker-directory"
            onClick={() => {
              if (selectedDirectory !== null) {
                state.chooseDirectory(null)
              }
            }}
            type="button"
          >
            <Folder aria-hidden="true" size={13} />
            {selectedDirectory?.path ?? 'Choose a directory'}
          </button>
          {selectedDirectory !== null ? (
            <input
              ref={firstMessageRef}
              aria-label="First message"
              className="new-session-picker-message"
              onChange={(event) => {
                state.setInitialMessage(event.currentTarget.value)
              }}
              onKeyDown={(event) => {
                if (
                  event.key === 'Backspace' &&
                  event.currentTarget.value === ''
                ) {
                  state.chooseDirectory(null)
                }
              }}
              placeholder="First message (optional)"
              value={state.initialMessage}
            />
          ) : null}
          <button
            aria-label="Close new session"
            className="new-session-picker-close"
            onClick={onClose}
            type="button"
          >
            <X aria-hidden="true" size={16} />
          </button>
        </header>
        {selectedDirectory === null ? (
          <SessionDirectoryOptions
            autoFocus={!isMobile}
            activeIndex={activeIndex}
            directories={state.directories}
            error={state.error}
            isLoading={state.isLoadingDirectories}
            onActiveIndexChange={setActiveIndex}
            onQueryChange={(query) => {
              state.setDirectoryQuery(query)
              setActiveIndex(0)
            }}
            onSelect={state.chooseDirectory}
            query={state.directoryQuery}
          />
        ) : (
          <div
            aria-label="Session options"
            className="new-session-picker-options"
            role="listbox"
          >
            <h2>START WITH</h2>
            <button
              aria-selected={state.engine === 'claude'}
              className="new-session-picker-option"
              onClick={() => {
                state.setEngine('claude')
              }}
              role="option"
              type="button"
            >
              <span>
                <strong>Claude Code</strong>
                <span> in {selectedDirectory.path}</span>
              </span>
              <kbd>↵</kbd>
            </button>
            <button
              aria-selected={state.engine === 'codex'}
              className="new-session-picker-option"
              onClick={() => {
                state.setEngine('codex')
              }}
              role="option"
              type="button"
            >
              <span>
                <strong>Codex</strong>
                <span> in {selectedDirectory.path}</span>
              </span>
              <kbd>⌥↵</kbd>
            </button>
            <h2>OPTIONS</h2>
            <button
              aria-pressed={state.worktree}
              className="new-session-picker-option new-session-picker-worktree"
              onClick={() => {
                state.setWorktree(!state.worktree)
              }}
              role="option"
              type="button"
            >
              <GitBranch aria-hidden="true" size={14} />
              <span>New worktree</span>
              <kbd>{state.worktree ? 'on' : 'off'}</kbd>
            </button>
            {state.error !== null ? (
              <p className="new-session-picker-error" role="alert">
                {state.error}
              </p>
            ) : null}
          </div>
        )}
        <footer className="new-session-picker-footer">
          {isMobile ? (
            <button
              className="new-session-picker-start"
              disabled={
                selectedDirectory === null || state.isStarting || launchPending
              }
              onClick={() => {
                if (!launchPending) void state.start(onLaunched)
              }}
              type="button"
            >
              {state.isStarting || launchPending
                ? 'Starting… Waiting for session'
                : `Start with ${state.engine === 'claude' ? 'Claude Code' : 'Codex'}`}
            </button>
          ) : (
            <>
              <span>↵ Start</span>
              <span>↑↓ Engine</span>
              <span>⌥W Worktree</span>
              <span>⌫ Change directory</span>
              <span className="new-session-picker-summary">
                {launchPending
                  ? 'Waiting for session feed…'
                  : state.isStarting
                    ? 'Starting session…'
                    : `${state.engine} · ${selectedDirectory?.path ?? 'choose directory'}${state.worktree ? ' · new worktree' : ''}`}
              </span>
            </>
          )}
        </footer>
      </section>
    </div>
  )
}
