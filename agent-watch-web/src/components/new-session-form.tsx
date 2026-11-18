import '#components/new-session.css'

import { ArrowLeft, ChevronDown, Folder, GitBranch, X } from 'lucide-react'
import { useState } from 'react'

import { NewSessionModelPicker } from '#components/new-session-model-picker'
import { SessionDirectoryOptions } from '#components/session-directory-options'
import {
  skillCompletionTextareaAttributes,
  SkillSuggestionList,
  useSkillCompletion,
} from '#components/skill-completion'
import type { NewSessionState } from '#use-new-session'

type NewSessionFormProps = {
  state: NewSessionState
  isMobile?: boolean
  initialDirectoryPickerOpen?: boolean
  initialModelPickerOpen?: boolean
  launchPending?: boolean
  onClose: () => void
  onLaunched: (launchId: string) => void
}

export function NewSessionForm({
  state,
  isMobile = false,
  initialDirectoryPickerOpen = false,
  initialModelPickerOpen = false,
  launchPending = false,
  onClose,
  onLaunched,
}: NewSessionFormProps) {
  const [directoryPickerOpen, setDirectoryPickerOpen] = useState(
    initialDirectoryPickerOpen,
  )
  const [engineMenuOpen, setEngineMenuOpen] = useState(false)
  const [modelPickerOpen, setModelPickerOpen] = useState(initialModelPickerOpen)
  const selectedDirectory = state.selectedDirectory
  const skillCompletion = useSkillCompletion({
    draft: state.initialMessage,
    engine: state.engine,
    skills: state.skillSuggestions,
    onDraftChange: state.setInitialMessage,
  })

  return (
    <section aria-label="New session" className="new-session-view">
      <header className="new-session-header">
        {isMobile ? (
          <button
            aria-label="Back to Inbox"
            className="new-session-back"
            onClick={onClose}
            type="button"
          >
            <ArrowLeft aria-hidden="true" size={22} />
          </button>
        ) : null}
        <h1>New session</h1>
        <button
          aria-expanded={directoryPickerOpen}
          aria-haspopup="dialog"
          className={`new-session-directory-button${selectedDirectory === null ? ' is-empty' : ''}`}
          onClick={() => {
            setDirectoryPickerOpen((open) => !open)
          }}
          type="button"
        >
          <Folder aria-hidden="true" size={14} />
          <span>{selectedDirectory?.path ?? 'Choose a directory'}</span>
          <ChevronDown aria-hidden="true" size={13} />
        </button>
        {!isMobile ? (
          <button
            aria-label="Close new session"
            className="new-session-close"
            onClick={onClose}
            type="button"
          >
            <X aria-hidden="true" size={16} />
          </button>
        ) : null}
        {directoryPickerOpen ? (
          <div
            aria-label="Choose a directory"
            className="new-session-directory-popover"
            role="dialog"
          >
            <SessionDirectoryOptions
              autoFocus
              directories={state.directories}
              error={state.error}
              isLoading={state.isLoadingDirectories}
              onQueryChange={state.setDirectoryQuery}
              onSelect={(directory) => {
                state.chooseDirectory(directory)
                setDirectoryPickerOpen(false)
              }}
              query={state.directoryQuery}
            />
          </div>
        ) : null}
      </header>
      <div className="new-session-placeholder" aria-live="polite">
        {launchPending
          ? 'Starting session… Waiting for it to appear in the session list.'
          : 'The conversation opens here when the session starts.'}
      </div>
      <div className="new-session-composer">
        <div className="new-session-composer-box">
          <textarea
            ref={skillCompletion.textareaRef}
            aria-label="First message"
            onChange={skillCompletion.handleChange}
            onSelect={skillCompletion.handleSelect}
            onKeyDown={skillCompletion.handleKeyDown}
            placeholder="First message (optional)"
            value={state.initialMessage}
            {...skillCompletionTextareaAttributes({
              isOpen: skillCompletion.isOpen,
              id: skillCompletion.suggestionsId,
              selectedIndex: skillCompletion.selectedSkillIndex,
            })}
          />
          {skillCompletion.isOpen ? (
            <SkillSuggestionList
              id={skillCompletion.suggestionsId}
              completion={skillCompletion.completion}
              skills={skillCompletion.matchingSkills}
              selectedIndex={skillCompletion.selectedSkillIndex}
              onSelect={skillCompletion.selectSkill}
            />
          ) : null}
          {state.modelChangeNotice !== null ? (
            <div className="new-session-model-change" role="status">
              <span>{state.modelChangeNotice}</span>
              <button onClick={state.undoEngineChange} type="button">
                Undo
              </button>
            </div>
          ) : null}
          <div className="new-session-composer-actions">
            <div className="new-session-engine-control">
              <button
                aria-expanded={engineMenuOpen}
                aria-haspopup="menu"
                className="new-session-control-button"
                onClick={() => {
                  setEngineMenuOpen((open) => !open)
                }}
                type="button"
              >
                {state.engine === 'claude' ? 'Claude Code' : 'Codex'}
                <ChevronDown aria-hidden="true" size={13} />
              </button>
              {engineMenuOpen ? (
                <div
                  aria-label="Choose an engine"
                  className="new-session-menu"
                  role="menu"
                >
                  {(['claude', 'codex'] as const).map((engine) => (
                    <button
                      key={engine}
                      aria-checked={state.engine === engine}
                      onClick={() => {
                        state.setEngine(engine)
                        setEngineMenuOpen(false)
                      }}
                      role="menuitemradio"
                      type="button"
                    >
                      {engine === 'claude' ? 'Claude Code' : 'Codex'}
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
            <NewSessionModelPicker
              activeModelOption={state.activeModelOption}
              engine={state.engine}
              error={state.modelOptionsError}
              isLoading={state.isLoadingModelOptions}
              isOpen={modelPickerOpen}
              onEffortChange={state.setSelectedEffort}
              onModelChange={state.chooseModel}
              onOpenChange={setModelPickerOpen}
              options={state.modelOptions}
              selectedEffort={state.selectedEffort}
              selectedModel={state.selectedModel}
              variant="control"
            />
            <button
              aria-pressed={state.worktree}
              className={`new-session-control-button new-session-worktree${state.worktree ? ' is-enabled' : ''}`}
              onClick={() => {
                state.setWorktree(!state.worktree)
              }}
              type="button"
            >
              <GitBranch aria-hidden="true" size={13} />
              New worktree
            </button>
            <span className="new-session-launch-status" role="status">
              {state.error ??
                (launchPending
                  ? 'Waiting for the session feed…'
                  : selectedDirectory === null
                    ? 'Choose a directory to start'
                    : selectedDirectory.path)}
            </span>
            <button
              className="new-session-start"
              disabled={
                selectedDirectory === null || state.isStarting || launchPending
              }
              onClick={() => {
                void state.start(onLaunched)
              }}
              type="button"
            >
              {state.isStarting || launchPending ? 'Starting…' : 'Start ↵'}
            </button>
          </div>
        </div>
      </div>
    </section>
  )
}
