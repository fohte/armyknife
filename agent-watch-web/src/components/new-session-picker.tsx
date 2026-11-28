import '#components/new-session.css'

import { Cpu, Folder, GitBranch, X } from 'lucide-react'
import {
  type KeyboardEvent as ReactKeyboardEvent,
  useEffect,
  useRef,
  useState,
} from 'react'

import { NewSessionModelPicker } from '#components/new-session-model-picker'
import { NewSessionRestoredNotice } from '#components/new-session-restored-notice'
import { SessionDirectoryOptions } from '#components/session-directory-options'
import type { LaunchSessionRequest } from '#new-session-api'
import type { NewSessionState } from '#use-new-session'

function isImeEnter(event: ReactKeyboardEvent) {
  if (event.key !== 'Enter') return false
  if (event.nativeEvent.isComposing) return true
  // Some browsers clear isComposing before dispatching the IME confirmation Enter.
  // eslint-disable-next-line @typescript-eslint/no-deprecated -- keyCode 229 identifies that IME event.
  return event.nativeEvent.keyCode === 229
}

type NewSessionPickerProps = {
  state: NewSessionState
  isMobile?: boolean
  initialModelPickerOpen?: boolean
  launchPending?: boolean
  now?: Date
  onClose: () => void
  onStart: (request: LaunchSessionRequest) => void
}

export function NewSessionPicker({
  state,
  isMobile = false,
  initialModelPickerOpen = false,
  launchPending = false,
  now,
  onClose,
  onStart,
}: NewSessionPickerProps) {
  const [activeIndex, setActiveIndex] = useState(0)
  const [modelPickerOpen, setModelPickerOpen] = useState(initialModelPickerOpen)
  const firstMessageRef = useRef<HTMLInputElement>(null)
  const modelOptionRef = useRef<HTMLButtonElement>(null)
  const pickerRef = useRef<HTMLElement>(null)
  const wasModelPickerOpen = useRef(false)
  const selectedDirectory = state.selectedDirectory
  const restoredNotice =
    state.restoredAt === null ? null : (
      <NewSessionRestoredNotice
        savedAt={state.restoredAt}
        {...(now === undefined ? {} : { now })}
        isMobile={isMobile}
        onClear={state.clearDraft}
      />
    )

  useEffect(() => {
    if (modelPickerOpen) {
      pickerRef.current?.focus()
    } else if (wasModelPickerOpen.current) {
      modelOptionRef.current?.focus()
    }
    wasModelPickerOpen.current = modelPickerOpen
  }, [modelPickerOpen])

  useEffect(() => {
    if (selectedDirectory !== null) firstMessageRef.current?.focus()
  }, [selectedDirectory])

  return (
    <div className="new-session-picker-overlay" onMouseDown={onClose}>
      <section
        aria-label="New session"
        aria-modal="true"
        className={`new-session-picker${isMobile ? ' is-mobile' : ''}`}
        ref={pickerRef}
        tabIndex={-1}
        onKeyDown={(event) => {
          if (isImeEnter(event)) return
          if (modelPickerOpen) {
            if (event.key === 'Escape' || event.key === 'Backspace') {
              event.preventDefault()
              setModelPickerOpen(false)
              return
            }
            if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
              event.preventDefault()
              const options = state.modelOptions?.models ?? []
              const models = [null, ...options.map((model) => model.id)]
              const currentIndex = models.indexOf(state.selectedModel)
              const nextIndex =
                (currentIndex +
                  (event.key === 'ArrowDown' ? 1 : -1) +
                  models.length) %
                models.length
              state.chooseModel(models[nextIndex] ?? null)
              return
            }
            if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
              event.preventDefault()
              const efforts = state.activeModelOption?.efforts ?? []
              const currentIndex =
                state.selectedEffort === null
                  ? 0
                  : efforts.indexOf(state.selectedEffort) + 1
              const nextIndex = Math.max(
                0,
                Math.min(
                  efforts.length,
                  currentIndex + (event.key === 'ArrowRight' ? 1 : -1),
                ),
              )
              state.setSelectedEffort(efforts[nextIndex - 1] ?? null)
              return
            }
            if (event.key === 'Enter') {
              if (event.target instanceof HTMLButtonElement) return
              event.preventDefault()
              setModelPickerOpen(false)
              return
            }
          }
          if (
            event.key === 'Enter' &&
            event.target instanceof HTMLButtonElement
          ) {
            return
          }
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
            state.start(onStart, event.altKey ? 'codex' : state.engine)
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
            onClick={() => {
              onClose()
            }}
            type="button"
          >
            <X aria-hidden="true" size={16} />
          </button>
        </header>
        {selectedDirectory !== null ? restoredNotice : null}
        {selectedDirectory === null ? (
          <SessionDirectoryOptions
            autoFocus={!isMobile}
            activeIndex={activeIndex}
            afterSearchInput={restoredNotice}
            directories={state.directories}
            error={state.error}
            isLoading={state.isLoadingDirectories}
            {...(now === undefined ? {} : { now })}
            onActiveIndexChange={setActiveIndex}
            onQueryChange={(query) => {
              state.setDirectoryQuery(query)
              setActiveIndex(0)
            }}
            onSelect={state.chooseDirectory}
            query={state.directoryQuery}
          />
        ) : modelPickerOpen ? (
          <NewSessionModelPicker
            activeModelOption={state.activeModelOption}
            engine={state.engine}
            error={state.modelOptionsError}
            isLoading={state.isLoadingModelOptions}
            isOpen
            onEffortChange={state.setSelectedEffort}
            onModelChange={state.chooseModel}
            onOpenChange={setModelPickerOpen}
            options={state.modelOptions}
            selectedEffort={state.selectedEffort}
            selectedModel={state.selectedModel}
            variant="palette"
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
              disabled={launchPending}
              onClick={() => {
                state.setEngine('claude')
              }}
              onKeyDown={(event) => {
                if (event.key !== 'Enter' || isImeEnter(event)) {
                  return
                }
                event.preventDefault()
                if (!launchPending) {
                  state.start(onStart, event.altKey ? 'codex' : 'claude')
                }
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
              disabled={launchPending}
              onClick={() => {
                state.setEngine('codex')
              }}
              onKeyDown={(event) => {
                if (event.key !== 'Enter' || isImeEnter(event)) {
                  return
                }
                event.preventDefault()
                if (!launchPending) {
                  state.start(onStart, 'codex')
                }
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
              aria-expanded={modelPickerOpen}
              className="new-session-picker-option new-session-picker-model"
              ref={modelOptionRef}
              onClick={() => {
                setModelPickerOpen(true)
              }}
              role="option"
              type="button"
            >
              <Cpu aria-hidden="true" size={14} />
              <span>Model · effort</span>
              <kbd>
                {state.selectedModel ??
                  state.modelOptions?.defaultModel ??
                  'default'}{' '}
                ·{' '}
                {state.selectedEffort ??
                  state.activeModelOption?.defaultEffort ??
                  'default'}
              </kbd>
            </button>
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
        {modelPickerOpen ? null : (
          <footer className="new-session-picker-footer">
            {isMobile ? (
              <button
                className="new-session-picker-start"
                disabled={selectedDirectory === null || launchPending}
                onClick={() => {
                  if (!launchPending) state.start(onStart, state.engine)
                }}
                type="button"
              >
                {launchPending
                  ? 'Starting… Waiting for session'
                  : `Start with ${state.engine === 'claude' ? 'Claude Code' : 'Codex'}`}
              </button>
            ) : (
              <>
                <span>↑↓ Engine / model</span>
                <span>⌥W Worktree</span>
                <span>⌫ Change directory</span>
                <button
                  aria-label={`Start session with ${state.engine === 'claude' ? 'Claude Code' : 'Codex'}`}
                  className="new-session-picker-start new-session-picker-start-desktop"
                  disabled={selectedDirectory === null || launchPending}
                  onClick={() => {
                    if (!launchPending) state.start(onStart, state.engine)
                  }}
                  type="button"
                >
                  ↵ Start
                </button>
                <span className="new-session-picker-summary">
                  {launchPending
                    ? 'Waiting for session feed…'
                    : `${state.engine} · ${selectedDirectory?.path ?? 'choose directory'}${state.worktree ? ' · new worktree' : ''}`}
                </span>
              </>
            )}
          </footer>
        )}
      </section>
    </div>
  )
}
