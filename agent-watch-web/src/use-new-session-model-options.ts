import { useCallback, useEffect, useRef, useState } from 'react'

import {
  getSessionModelOptions,
  type SessionEngine,
  type SessionModelOption,
  type SessionModelOptions,
} from '#new-session-api'
import {
  defaultEffortModelOption,
  type NewSessionModelSelection,
  reconcileModelSelection,
} from '#new-session-selection'

type ModelOptionsState = {
  modelOptions: SessionModelOptions | null
  isLoading: boolean
  error: string | null
}

type EngineOptionsState = ModelOptionsState & {
  selectedModel: string | null
  selectedEffort: string | null
  activeModelOption: SessionModelOption | undefined
}

type UpdateSelection = (
  engine: SessionEngine,
  selection: NewSessionModelSelection,
  persistDraft?: boolean,
) => void

const emptyModelOptionsState = (): Record<
  SessionEngine,
  ModelOptionsState
> => ({
  claude: { modelOptions: null, isLoading: false, error: null },
  codex: { modelOptions: null, isLoading: false, error: null },
})

export function useNewSessionModelOptions(
  active: boolean,
  selections: Record<SessionEngine, NewSessionModelSelection>,
  updateSelection: UpdateSelection,
) {
  const [modelOptionsByEngine, setModelOptionsByEngine] = useState(
    emptyModelOptionsState,
  )
  const modelOptionsByEngineRef = useRef(modelOptionsByEngine)
  modelOptionsByEngineRef.current = modelOptionsByEngine
  const selectionsRef = useRef(selections)
  selectionsRef.current = selections
  const activeRef = useRef(active)
  activeRef.current = active
  const requestSequenceRef = useRef<Record<SessionEngine, number>>({
    claude: 0,
    codex: 0,
  })

  const invalidateRequests = useCallback(() => {
    requestSequenceRef.current = {
      claude: requestSequenceRef.current.claude + 1,
      codex: requestSequenceRef.current.codex + 1,
    }
  }, [])

  const loadModelOptionsFor = useCallback(
    (engine: SessionEngine) => {
      const sequence = requestSequenceRef.current[engine] + 1
      requestSequenceRef.current[engine] = sequence
      setModelOptionsByEngine((states) => ({
        ...states,
        [engine]: { ...states[engine], isLoading: true, error: null },
      }))
      void getSessionModelOptions(engine).match(
        (options) => {
          if (
            !activeRef.current ||
            requestSequenceRef.current[engine] !== sequence
          ) {
            return
          }
          setModelOptionsByEngine((states) => ({
            ...states,
            [engine]: { modelOptions: options, isLoading: false, error: null },
          }))
          updateSelection(
            engine,
            reconcileModelSelection(selectionsRef.current[engine], options),
            false,
          )
        },
        (requestError) => {
          if (
            !activeRef.current ||
            requestSequenceRef.current[engine] !== sequence
          ) {
            return
          }
          setModelOptionsByEngine((states) => ({
            ...states,
            [engine]: {
              ...states[engine],
              isLoading: false,
              error: requestError.message,
            },
          }))
        },
      )
    },
    [updateSelection],
  )

  useEffect(() => {
    if (!active) {
      invalidateRequests()
      setModelOptionsByEngine(emptyModelOptionsState())
      return
    }
    for (const engine of ['claude', 'codex'] as const) {
      loadModelOptionsFor(engine)
    }
    return invalidateRequests
  }, [active, invalidateRequests, loadModelOptionsFor])

  const retryFailedModelOptionsFor = useCallback(
    (engine: SessionEngine) => {
      if (
        activeRef.current &&
        modelOptionsByEngineRef.current[engine].error !== null
      ) {
        loadModelOptionsFor(engine)
      }
    },
    [loadModelOptionsFor],
  )

  const resetModelOptions = useCallback(() => {
    invalidateRequests()
    setModelOptionsByEngine(emptyModelOptionsState())
  }, [invalidateRequests])

  const chooseModelFor = useCallback(
    (engine: SessionEngine, model: string | null) => {
      const selection = selectionsRef.current[engine]
      const options = modelOptionsByEngineRef.current[engine].modelOptions
      const target =
        model === null
          ? options === null
            ? undefined
            : defaultEffortModelOption(options)
          : options?.models.find((option) => option.id === model)
      const effort =
        selection.effort !== null &&
        target !== undefined &&
        !target.efforts.includes(selection.effort)
          ? null
          : selection.effort
      updateSelection(engine, { model, effort })
    },
    [updateSelection],
  )

  const chooseEffortFor = useCallback(
    (engine: SessionEngine, effort: string | null) => {
      updateSelection(engine, { ...selectionsRef.current[engine], effort })
    },
    [updateSelection],
  )

  const getEngineOptions = (engine: SessionEngine): EngineOptionsState => {
    const { modelOptions, isLoading, error } = modelOptionsByEngine[engine]
    const { model: selectedModel, effort: selectedEffort } = selections[engine]
    const activeModel = modelOptions?.models.find(
      (option) => option.id === selectedModel,
    )
    const defaultModel =
      modelOptions === null ? undefined : defaultEffortModelOption(modelOptions)
    return {
      modelOptions,
      isLoading,
      error,
      selectedModel,
      selectedEffort,
      activeModelOption: selectedModel === null ? defaultModel : activeModel,
    }
  }

  const engineOptions: Record<SessionEngine, EngineOptionsState> = {
    claude: getEngineOptions('claude'),
    codex: getEngineOptions('codex'),
  }

  return {
    engineOptions,
    chooseModelFor,
    chooseEffortFor,
    retryFailedModelOptionsFor,
    resetModelOptions,
  }
}
