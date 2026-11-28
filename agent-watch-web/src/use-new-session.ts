import { useCallback, useEffect, useRef, useState } from 'react'

import {
  getSessionDirectories,
  getSessionModelOptions,
  getSessionSkillSuggestions,
  launchSession,
  type SessionDirectory,
  type SessionEngine,
  type SessionModelOptions,
} from '#new-session-api'
import {
  defaultEffortModelOption,
  type NewSessionModelSelection,
  reconcileModelSelection,
} from '#new-session-selection'
import type { SkillSuggestion } from '#types'

export function useNewSession(active: boolean) {
  const [directoryQuery, setDirectoryQuery] = useState('')
  const [directories, setDirectories] = useState<SessionDirectory[]>([])
  const [selectedDirectory, setSelectedDirectory] =
    useState<SessionDirectory | null>(null)
  const [engine, setEngine] = useState<SessionEngine>('claude')
  const [modelOptionsCache, setModelOptionsCache] = useState<
    Partial<Record<SessionEngine, SessionModelOptions>>
  >({})
  const [isLoadingModelOptions, setIsLoadingModelOptions] = useState(false)
  const [modelOptionsError, setModelOptionsError] = useState<string | null>(
    null,
  )
  const [selections, setSelections] = useState<
    Record<SessionEngine, NewSessionModelSelection>
  >({
    claude: { model: null, effort: null },
    codex: { model: null, effort: null },
  })
  const [worktree, setWorktree] = useState(false)
  const [initialMessage, setInitialMessage] = useState('')
  const [skillSuggestions, setSkillSuggestions] = useState<SkillSuggestion[]>(
    [],
  )
  const [isLoadingDirectories, setIsLoadingDirectories] = useState(false)
  const [isStarting, setIsStarting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const requestSequence = useRef(0)
  const skillSuggestionsSequence = useRef(0)
  const launchSequence = useRef(0)
  const selectionsRef = useRef(selections)
  selectionsRef.current = selections

  const updateSelection = useCallback(
    (selectionEngine: SessionEngine, selection: NewSessionModelSelection) => {
      const nextSelections = {
        ...selectionsRef.current,
        [selectionEngine]: selection,
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
        updateSelection(engine, reconciled)
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

  const reset = useCallback(() => {
    requestSequence.current += 1
    skillSuggestionsSequence.current += 1
    launchSequence.current += 1
    setDirectoryQuery('')
    setDirectories([])
    setSelectedDirectory(null)
    setEngine('claude')
    setModelOptionsCache({})
    setIsLoadingModelOptions(false)
    setModelOptionsError(null)
    const emptySelections = {
      claude: { model: null, effort: null },
      codex: { model: null, effort: null },
    }
    selectionsRef.current = emptySelections
    setSelections(emptySelections)
    setWorktree(false)
    setInitialMessage('')
    setSkillSuggestions([])
    setIsLoadingDirectories(false)
    setIsStarting(false)
    setError(null)
  }, [])

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
    async (
      onLaunched: (launchId: string) => void,
      engineOverride?: SessionEngine,
    ) => {
      if (selectedDirectory === null || isStarting) return
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
      const sequence = launchSequence.current + 1
      launchSequence.current = sequence
      setError(null)
      setIsStarting(true)
      const result = await launchSession({
        directory: selectedDirectory.path,
        engine: launchEngine,
        worktree,
        ...(launchModel === null ? {} : { model: launchModel }),
        ...(launchEffort === null ? {} : { reasoningEffort: launchEffort }),
        initialMessage,
      })
      if (launchSequence.current !== sequence) return
      result.match(
        ({ launchId }) => {
          onLaunched(launchId)
        },
        (launchError) => {
          setError(launchError.message)
        },
      )
      setIsStarting(false)
    },
    [
      engine,
      initialMessage,
      isStarting,
      modelOptionsCache,
      selectedDirectory,
      worktree,
    ],
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
    skillSuggestions,
    isLoadingDirectories,
    isStarting,
    error,
    setDirectoryQuery,
    chooseDirectory,
    setEngine: changeEngine,
    chooseModel,
    setSelectedEffort: chooseEffort,
    setWorktree,
    setInitialMessage,
    start,
    reset,
  }
}

export type NewSessionState = ReturnType<typeof useNewSession>
