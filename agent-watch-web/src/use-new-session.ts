import { useCallback, useEffect, useRef, useState } from 'react'

import { useDraft } from '#components/use-composer-draft'
import {
  getSessionDirectories,
  getSessionModelOptions,
  getSessionSkillSuggestions,
  type LaunchSessionRequest,
  type SessionDirectory,
  type SessionEngine,
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
  const [modelOptionsCache, setModelOptionsCache] = useState<
    Partial<Record<SessionEngine, SessionModelOptions>>
  >({})
  const [isLoadingModelOptions, setIsLoadingModelOptions] = useState(false)
  const [modelOptionsError, setModelOptionsError] = useState<string | null>(
    null,
  )
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
      setModelOptionsCache({})
      setIsLoadingModelOptions(false)
      return
    }
    let current = true
    setModelOptionsError(null)
    setIsLoadingModelOptions(true)
    void getSessionModelOptions(engine).match(
      (options) => {
        if (!current) return
        setModelOptionsCache((cache) => ({ ...cache, [engine]: options }))
        setIsLoadingModelOptions(false)
        const reconciled = reconcileModelSelection(
          selectionsRef.current[engine],
          options,
        )
        updateSelection(engine, reconciled, false)
      },
      (error) => {
        if (!current) return
        setIsLoadingModelOptions(false)
        setModelOptionsError(error.message)
      },
    )
    return () => {
      current = false
    }
  }, [active, engine, updateSelection])

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
    setModelOptionsCache({})
    setIsLoadingModelOptions(false)
    setModelOptionsError(null)
    setSkillSuggestions([])
    setIsLoadingDirectories(false)
  }, [clearDraft])

  const changeEngine = useCallback((nextEngine: SessionEngine) => {
    setEngine(nextEngine)
  }, [])

  const chooseModel = useCallback(
    (model: string | null) => {
      const selection = selectionsRef.current[engine]
      const options = modelOptionsCache[engine]
      const target =
        model === null
          ? options === undefined
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
    [engine, modelOptionsCache, updateSelection],
  )

  const chooseEffort = useCallback(
    (effort: string | null) => {
      updateSelection(engine, { ...selectionsRef.current[engine], effort })
    },
    [engine, updateSelection],
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
      const launchOptions = modelOptionsCache[launchEngine]
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
    [engine, initialMessage, modelOptionsCache, selectedDirectory, worktree],
  )

  const modelOptions = modelOptionsCache[engine] ?? null
  const { model: selectedModel, effort: selectedEffort } = selections[engine]
  const activeModel = modelOptions?.models.find(
    (option) => option.id === selectedModel,
  )
  const defaultModel =
    modelOptions === null ? undefined : defaultEffortModelOption(modelOptions)
  const activeModelOption = selectedModel === null ? defaultModel : activeModel

  return {
    directoryQuery,
    directories,
    selectedDirectory,
    engine,
    modelOptions,
    isLoadingModelOptions,
    modelOptionsError,
    selectedModel,
    selectedEffort,
    activeModelOption,
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
    setWorktree,
    setInitialMessage,
    start,
    clearDraft,
    reset,
  }
}

export type NewSessionState = ReturnType<typeof useNewSession>
