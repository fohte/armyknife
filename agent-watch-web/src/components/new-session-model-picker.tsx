import '#components/new-session.css'
import '#components/new-session-model.css'

import { Cpu, X } from 'lucide-react'
import { type CSSProperties, useRef, useState } from 'react'

import { useDismissableLayer } from '#components/use-dismissable-layer'
import type {
  SessionEngine,
  SessionModelOption,
  SessionModelOptions,
} from '#new-session-api'

type NewSessionModelPickerProps = {
  variant: 'control' | 'palette'
  engine: SessionEngine
  options: SessionModelOptions | null
  activeModelOption: SessionModelOption | undefined
  selectedModel: string | null
  selectedEffort: string | null
  isOpen: boolean
  isLoading: boolean
  error: string | null
  onOpenChange: (open: boolean) => void
  onModelChange: (model: string | null) => void
  onEffortChange: (effort: string | null) => void
}

function effectiveModelLabel(
  options: SessionModelOptions | null,
  selectedModel: string | null,
  engine: SessionEngine,
) {
  if (selectedModel !== null) return selectedModel
  if (options?.defaultModel !== null && options?.defaultModel !== undefined) {
    return options.defaultModel
  }
  return engine === 'claude' ? 'Claude Code default' : 'Codex default'
}

function ModelChoices({
  options,
  activeModelOption,
  selectedModel,
  selectedEffort,
  onModelChange,
  onEffortChange,
  engine,
  isLoading,
  error,
}: Omit<NewSessionModelPickerProps, 'variant' | 'isOpen' | 'onOpenChange'>) {
  const modelLabel = effectiveModelLabel(options, null, engine)
  const models = options?.models ?? []

  return (
    <div className="new-session-model-picker-content">
      <h2>{`MODEL · ${engine === 'claude' ? 'CLAUDE CODE' : 'CODEX'}`}</h2>
      <div
        aria-label="Model"
        className="new-session-model-list"
        role="radiogroup"
      >
        <button
          aria-checked={selectedModel === null}
          className="new-session-model-option"
          onClick={() => {
            onModelChange(null)
          }}
          role="radio"
          type="button"
        >
          <span aria-hidden="true" className="new-session-model-check" />
          <span>
            Default <small>profile default</small>
          </span>
          <span className="new-session-model-detail">{modelLabel}</span>
        </button>
        {models.map((model) => (
          <button
            key={model.id}
            aria-checked={selectedModel === model.id}
            className={`new-session-model-option${selectedModel === model.id ? ' is-selected' : ''}`}
            onClick={() => {
              onModelChange(model.id)
            }}
            role="radio"
            type="button"
          >
            <span aria-hidden="true" className="new-session-model-check" />
            <span>{model.label}</span>
            <span className="new-session-model-detail">
              {model.defaultEffort === null
                ? ''
                : `up to ${model.efforts.at(-1) ?? model.defaultEffort}`}
            </span>
          </button>
        ))}
      </div>
      <h2 className="new-session-model-effort-heading">EFFORT</h2>
      {activeModelOption === undefined ? (
        <p
          className="new-session-model-state"
          role={error === null ? 'status' : 'alert'}
        >
          {error ??
            (isLoading
              ? 'Loading model options…'
              : 'Effort options are unavailable.')}
        </p>
      ) : (
        <div
          aria-label="Reasoning effort"
          className="new-session-effort-list"
          role="radiogroup"
        >
          <button
            aria-checked={selectedEffort === null}
            className="new-session-effort-option"
            onClick={() => {
              onEffortChange(null)
            }}
            role="radio"
            type="button"
          >
            {`Default · ${activeModelOption.defaultEffort ?? 'profile'}`}
          </button>
          {activeModelOption.efforts.map((effort) => (
            <button
              key={effort}
              aria-checked={selectedEffort === effort}
              className="new-session-effort-option"
              onClick={() => {
                onEffortChange(effort)
              }}
              role="radio"
              type="button"
            >
              {effort}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

export function NewSessionModelPicker(props: NewSessionModelPickerProps) {
  const {
    variant,
    engine,
    options,
    activeModelOption,
    selectedModel,
    selectedEffort,
    isOpen,
    isLoading,
    error,
    onOpenChange,
    onModelChange,
    onEffortChange,
  } = props
  const modelLabel = effectiveModelLabel(options, selectedModel, engine)
  const effortLabel =
    selectedEffort ??
    activeModelOption?.defaultEffort ??
    options?.defaultEffort ??
    'default'
  const triggerRef = useRef<HTMLButtonElement>(null)
  const controlRef = useRef<HTMLDivElement>(null)
  const [popoverPosition, setPopoverPosition] = useState<CSSProperties>({})
  useDismissableLayer(variant === 'control' && isOpen, onOpenChange, [
    controlRef,
  ])

  if (variant === 'palette') {
    return (
      <section
        aria-label="Model and reasoning effort"
        className="new-session-model-picker is-palette"
      >
        <header>
          <span>Model · effort</span>
          <button
            aria-label="Back to new session options"
            onClick={() => {
              onOpenChange(false)
            }}
            type="button"
          >
            <X aria-hidden="true" size={15} />
          </button>
        </header>
        <ModelChoices
          {...props}
          options={options}
          activeModelOption={activeModelOption}
          selectedModel={selectedModel}
          selectedEffort={selectedEffort}
          onModelChange={onModelChange}
          onEffortChange={onEffortChange}
          engine={engine}
          isLoading={isLoading}
          error={error}
        />
        <footer>
          ↑↓ Model <span>←→ Effort</span>{' '}
          <button
            onClick={() => {
              onOpenChange(false)
            }}
            type="button"
          >
            ↵ Done
          </button>
        </footer>
      </section>
    )
  }

  return (
    <div ref={controlRef} className="new-session-model-control">
      <button
        aria-expanded={isOpen}
        aria-haspopup="dialog"
        aria-label="Model and reasoning effort"
        className="new-session-control-button new-session-model-trigger"
        ref={triggerRef}
        onClick={() => {
          if (!isOpen) {
            const bounds = triggerRef.current?.getBoundingClientRect()
            if (bounds !== undefined) {
              const width = Math.min(360, window.innerWidth - 32)
              setPopoverPosition({
                bottom: window.innerHeight - bounds.top + 8,
                left: Math.max(
                  16,
                  Math.min(bounds.left, window.innerWidth - width - 16),
                ),
              })
            }
          }
          onOpenChange(!isOpen)
        }}
        type="button"
      >
        <Cpu aria-hidden="true" size={13} />
        <span>{modelLabel}</span>
        <span aria-hidden="true">·</span>
        <span>{effortLabel}</span>
        {selectedModel === null && selectedEffort === null ? (
          <small>default</small>
        ) : null}
        <span aria-hidden="true">▾</span>
      </button>
      {isOpen ? (
        <div
          aria-label="Model and reasoning effort"
          className="new-session-model-popover"
          role="dialog"
          style={popoverPosition}
        >
          <ModelChoices
            {...props}
            options={options}
            activeModelOption={activeModelOption}
            selectedModel={selectedModel}
            selectedEffort={selectedEffort}
            onModelChange={onModelChange}
            onEffortChange={onEffortChange}
            engine={engine}
            isLoading={isLoading}
            error={error}
          />
        </div>
      ) : null}
    </div>
  )
}
