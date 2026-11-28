import { useCallback, useEffect, useRef, useState } from 'react'

import { useDraft } from '#components/use-composer-draft'
import {
  getSessionDirectories,
  getSessionModelOptions,
  getSessionSkillSuggestions,
  type LaunchSessionRequest,
  type SessionDirectory,
  type SessionEngine,
  type SessionModelOption,
  type SessionModelOptions,
} from '#new-session-api'
import {
  decodeNewSessionDraft,
  emptyNewSessionDraftFields,
  encodeNewSessionDraft,
  isDefaultNewSessionDraft,
  NEW_SESSION_DRAFT_STORAGE_KEY,
  type NewSessionDraftFields,
} from '#new-session-draft'
import {
  defaultEffortModelOption,
  type NewSessionModelSelection,
  reconcileModelSelection,
} from '#new-session-selection'
import type { SkillSuggestion } from '#types'

export function useNewSession(active: boolean) {
  const { draft: storedDraft, updateDraft } = useDraft(
    NEW_SESSION_DRAFT_STORAGE_KEY,
  )
  const [initialDraft] = useState(() => decodeNewSessionDraft(storedDraft))
  const [directoryQuery, setDirectoryQuery] = useState(
    () => initialDraft?.directoryQuery ?? '',
  )
  const [directories, setDirectories] = useState<SessionDirectory[]>([])
  const [selectedDirectory, setSelectedDirectory] =
    useState<SessionDirectory | null>(() => initialDraft?.directory ?? null)
  const [engine, setEngine] = useState<SessionEngine>(
    () => initialDraft?.engine ?? 'claude',
  )
  const [modelOptionsByEngine, setModelOptionsByEngine] = useState<
    Record<
      SessionEngine,
      {
        modelOptions: SessionModelOptions | null
        isLoading: boolean
        error: string | null
      }
    >
  >({
    claude: { modelOptions: null, isLoading: false, error: null },
    codex: { modelOptions: null, isLoading: false, error: null },
  })
  const [selections, setSelections] = useState<
    Record<SessionEngine, NewSessionModelSelection>
  >(
    () =>
      initialDraft?.selections ?? {
        claude: { model: null, effort: null },
        codex: { model: null, effort: null },
      },
  )
  const [worktree, setWorktree] = useState(
    () => initialDraft?.worktree ?? false,
  )
  const [initialMessage, setInitialMessage] = useState(
    () => initialDraft?.initialMessage ?? '',
  )
  const [skillSuggestions, setSkillSuggestions] = useState<SkillSuggestion[]>(
    [],
  )
  const [isLoadingDirectories, setIsLoadingDirectories] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [restoredAt, setRestoredAt] = useState<string | null>(null)
  const activeRef = useRef(false)
  const lastFieldsRef = useRef(
    JSON.stringify(
      initialDraft === null
        ? emptyNewSessionDraftFields()
        : {
            directory: initialDraft.directory,
            directoryQuery: initialDraft.directoryQuery,
            engine: initialDraft.engine,
            selections: initialDraft.selections,
            worktree: initialDraft.worktree,
            initialMessage: initialDraft.initialMessage,
          },
    ),
  )
  const requestSequence = useRef(0)
  const skillSuggestionsSequence = useRef(0)
  const selectionsRef = useRef(selections)
  selectionsRef.current = selections
  const currentFields: NewSessionDraftFields = {
    directory: selectedDirectory,
    directoryQuery,
    engine,
    selections,
    worktree,
    initialMessage,
  }
  const draftFieldsRef = useRef(currentFields)
  draftFieldsRef.current = currentFields

  useEffect(() => {
    if (active && !activeRef.current) {
      setRestoredAt(decodeNewSessionDraft(storedDraft)?.savedAt ?? null)
    }
    activeRef.current = active
  }, [active, storedDraft])

  useEffect(() => {
    const fields: NewSessionDraftFields = {
      directory: selectedDirectory,
      directoryQuery,
      engine,
      selections,
      worktree,
      initialMessage,
    }
    const serializedFields = JSON.stringify(fields)
    if (serializedFields === lastFieldsRef.current) return
    lastFieldsRef.current = serializedFields
    if (isDefaultNewSessionDraft(fields)) {
      updateDraft('')
      return
    }
    const savedAt = new Date().toISOString()
    updateDraft(encodeNewSessionDraft(fields, savedAt))
  }, [
    directoryQuery,
    engine,
    initialMessage,
    selectedDirectory,
    selections,
    updateDraft,
    worktree,
  ])

  const updateSelection = useCallback(
    (
      selectionEngine: SessionEngine,
      selection: NewSessionModelSelection,
      persistDraft = true,
    ) => {
      const nextSelections = {
        ...selectionsRef.current,
        [selectionEngine]: selection,
      }
      if (!persistDraft) {
        lastFieldsRef.current = JSON.stringify({
          ...draftFieldsRef.current,
          selections: nextSelections,
        })
      }
      selectionsRef.current = nextSelections
      setSelections(nextSelections)
    },
    [],
  )

  useEffect(() => {
    if (!active) {
      setModelOptionsByEngine({
        claude: { modelOptions: null, isLoading: false, error: null },
        codex: { modelOptions: null, isLoading: false, error: null },
      })
      return
    }
    let current = true
    const engines: SessionEngine[] = ['claude', 'codex']
    setModelOptionsByEngine({
      claude: { modelOptions: null, isLoading: true, error: null },
      codex: { modelOptions: null, isLoading: true, error: null },
    })
    for (const optionEngine of engines) {
      void getSessionModelOptions(optionEngine).match(
        (options) => {
          if (!current) return
          setModelOptionsByEngine((states) => ({
            ...states,
            [optionEngine]: {
              modelOptions: options,
              isLoading: false,
              error: null,
            },
          }))
          const reconciled = reconcileModelSelection(
            selectionsRef.current[optionEngine],
            options,
          )
          updateSelection(optionEngine, reconciled, false)
        },
        (requestError) => {
          if (!current) return
          setModelOptionsByEngine((states) => ({
            ...states,
            [optionEngine]: {
              ...states[optionEngine],
              isLoading: false,
              error: requestError.message,
            },
          }))
        },
      )
    }
    return () => {
      current = false
    }
  }, [active, updateSelection])

  useEffect(() => {
    if (!active) return
    setError(null)
    const sequence = requestSequence.current + 1
    requestSequence.current = sequence
    setIsLoadingDirectories(true)
    const timeout = window.setTimeout(() => {
      void getSessionDirectories(directoryQuery).match(
        (nextDirectories) => {
          if (requestSequence.current !== sequence) return
          setDirectories(nextDirectories)
          setIsLoadingDirectories(false)
        },
        (requestError) => {
          if (requestSequence.current !== sequence) return
          setDirectories([])
          setError(requestError.message)
          setIsLoadingDirectories(false)
        },
      )
    }, 150)
    return () => {
      window.clearTimeout(timeout)
      requestSequence.current += 1
    }
  }, [active, directoryQuery])

  useEffect(() => {
    if (!active) {
      skillSuggestionsSequence.current += 1
      setSkillSuggestions([])
      return
    }
    const sequence = skillSuggestionsSequence.current + 1
    skillSuggestionsSequence.current = sequence
    setSkillSuggestions([])
    void getSessionSkillSuggestions(
      engine,
      selectedDirectory?.path ?? null,
    ).match(
      (skills) => {
        if (skillSuggestionsSequence.current === sequence) {
          setSkillSuggestions(skills)
        }
      },
      () => {
        if (skillSuggestionsSequence.current === sequence) {
          setSkillSuggestions([])
        }
      },
    )
    return () => {
      skillSuggestionsSequence.current += 1
    }
  }, [active, engine, selectedDirectory?.path])

  const clearDraft = useCallback(() => {
    const emptyFields = emptyNewSessionDraftFields()
    lastFieldsRef.current = JSON.stringify(emptyFields)
    updateDraft('')
    setDirectoryQuery('')
    setSelectedDirectory(null)
    setEngine('claude')
    const emptySelections = {
      claude: { model: null, effort: null },
      codex: { model: null, effort: null },
    }
    selectionsRef.current = emptySelections
    setSelections(emptySelections)
    setWorktree(false)
    setInitialMessage('')
    setError(null)
    setRestoredAt(null)
  }, [updateDraft])

  const reset = useCallback(() => {
    requestSequence.current += 1
    skillSuggestionsSequence.current += 1
    clearDraft()
    setDirectories([])
    setModelOptionsByEngine({
      claude: { modelOptions: null, isLoading: false, error: null },
      codex: { modelOptions: null, isLoading: false, error: null },
    })
    setSkillSuggestions([])
    setIsLoadingDirectories(false)
  }, [clearDraft])

  const changeEngine = useCallback((nextEngine: SessionEngine) => {
    setEngine(nextEngine)
  }, [])

  const chooseModelFor = useCallback(
    (selectionEngine: SessionEngine, model: string | null) => {
      const selection = selectionsRef.current[selectionEngine]
      const options = modelOptionsByEngine[selectionEngine].modelOptions
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
      updateSelection(selectionEngine, { model, effort })
    },
    [modelOptionsByEngine, updateSelection],
  )

  const chooseEffortFor = useCallback(
    (selectionEngine: SessionEngine, effort: string | null) => {
      updateSelection(selectionEngine, {
        ...selectionsRef.current[selectionEngine],
        effort,
      })
    },
    [updateSelection],
  )

  const chooseModel = useCallback(
    (model: string | null) => {
      chooseModelFor(engine, model)
    },
    [chooseModelFor, engine],
  )

  const chooseEffort = useCallback(
    (effort: string | null) => {
      chooseEffortFor(engine, effort)
    },
    [chooseEffortFor, engine],
  )

  const chooseDirectory = useCallback((directory: SessionDirectory | null) => {
    setSelectedDirectory(directory)
    if (directory !== null) setDirectoryQuery('')
    setError(null)
  }, [])

  const start = useCallback(
    (
      onStart: (request: LaunchSessionRequest) => void,
      engineOverride?: SessionEngine,
    ) => {
      if (selectedDirectory === null) return
      const launchEngine = engineOverride ?? engine
      const launchSelection = selectionsRef.current[launchEngine]
      const launchModel = launchSelection.model
      const launchOptions = modelOptionsByEngine[launchEngine].modelOptions
      const chosenModel = launchOptions?.models.find(
        (option) => option.id === launchModel,
      )
      const launchEffort =
        launchSelection.effort ??
        (launchModel === null ? null : (chosenModel?.defaultEffort ?? null))
      setError(null)
      onStart({
        directory: selectedDirectory.path,
        engine: launchEngine,
        worktree,
        ...(launchModel === null ? {} : { model: launchModel }),
        ...(launchEffort === null ? {} : { reasoningEffort: launchEffort }),
        initialMessage,
      })
    },
    [engine, initialMessage, modelOptionsByEngine, selectedDirectory, worktree],
  )

  const getEngineOptions = (
    optionEngine: SessionEngine,
  ): {
    modelOptions: SessionModelOptions | null
    isLoading: boolean
    error: string | null
    selectedModel: string | null
    selectedEffort: string | null
    activeModelOption: SessionModelOption | undefined
  } => {
    const { modelOptions, isLoading, error } =
      modelOptionsByEngine[optionEngine]
    const { model: selectedModel, effort: selectedEffort } =
      selections[optionEngine]
    const activeModel = modelOptions?.models.find(
      (option) => option.id === selectedModel,
    )
    const defaultModel =
      modelOptions === null ? undefined : defaultEffortModelOption(modelOptions)
    const activeModelOption =
      selectedModel === null ? defaultModel : activeModel
    return {
      modelOptions,
      isLoading,
      error,
      selectedModel,
      selectedEffort,
      activeModelOption,
    }
  }
  const engineOptions: Record<
    SessionEngine,
    ReturnType<typeof getEngineOptions>
  > = {
    claude: getEngineOptions('claude'),
    codex: getEngineOptions('codex'),
  }
  const currentEngineOptions = engineOptions[engine]

  return {
    directoryQuery,
    directories,
    selectedDirectory,
    engine,
    engineOptions,
    modelOptions: currentEngineOptions.modelOptions,
    isLoadingModelOptions: currentEngineOptions.isLoading,
    modelOptionsError: currentEngineOptions.error,
    selectedModel: currentEngineOptions.selectedModel,
    selectedEffort: currentEngineOptions.selectedEffort,
    activeModelOption: currentEngineOptions.activeModelOption,
    worktree,
    initialMessage,
    restoredAt,
    skillSuggestions,
    isLoadingDirectories,
    error,
    setLaunchError: setError,
    setDirectoryQuery,
    chooseDirectory,
    setEngine: changeEngine,
    chooseModel,
    setSelectedEffort: chooseEffort,
    chooseModelFor,
    chooseEffortFor,
    setWorktree,
    setInitialMessage,
    start,
    clearDraft,
    reset,
  }
}

export type NewSessionState = ReturnType<typeof useNewSession>
