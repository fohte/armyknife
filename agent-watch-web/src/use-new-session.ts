import { useCallback, useEffect, useRef, useState } from 'react'

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
  const [modelOptions, setModelOptions] = useState<SessionModelOptions | null>(
    null,
  )
  const [isLoadingModelOptions, setIsLoadingModelOptions] = useState(false)
  const [modelOptionsError, setModelOptionsError] = useState<string | null>(
    null,
  )
  const [selectedModel, setSelectedModel] = useState<string | null>(null)
  const [selectedEffort, setSelectedEffort] = useState<string | null>(null)
  const [modelChangeNotice, setModelChangeNotice] = useState<string | null>(
    null,
  )
  const [worktree, setWorktree] = useState(false)
  const [initialMessage, setInitialMessage] = useState('')
  const [skillSuggestions, setSkillSuggestions] = useState<SkillSuggestion[]>(
    [],
  )
  const [isLoadingDirectories, setIsLoadingDirectories] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const requestSequence = useRef(0)
  const skillSuggestionsSequence = useRef(0)
  const engineTransition = useRef<{
    engine: SessionEngine
    selection: NewSessionModelSelection
    previousEngine: SessionEngine
  } | null>(null)
  const engineUndo = useRef<{
    engine: SessionEngine
    selection: NewSessionModelSelection
    previousEngine: SessionEngine
  } | null>(null)
  const selectionRef = useRef<NewSessionModelSelection>({
    model: null,
    effort: null,
  })
  selectionRef.current = { model: selectedModel, effort: selectedEffort }

  useEffect(() => {
    if (!active) {
      setModelOptions(null)
      setIsLoadingModelOptions(false)
      return
    }
    let current = true
    setModelOptionsError(null)
    setIsLoadingModelOptions(true)
    void getSessionModelOptions(engine).match(
      (options) => {
        if (!current) return
        setModelOptions(options)
        setIsLoadingModelOptions(false)
        const transition = engineTransition.current
        if (transition?.engine !== engine) return
        const reconciled = reconcileModelSelection(
          transition.selection,
          options,
        )
        selectionRef.current = {
          model: reconciled.model,
          effort: reconciled.effort,
        }
        setSelectedModel(reconciled.model)
        setSelectedEffort(reconciled.effort)
        if (reconciled.modelChanged || reconciled.effortChanged) {
          const modelMessage = reconciled.modelChanged
            ? `${engine === 'claude' ? 'Claude Code' : 'Codex'} has no ${transition.selection.model ?? 'selected model'}, so the model is back to the default.`
            : ''
          const effortMessage = reconciled.effortChanged
            ? `${transition.selection.effort ?? 'selected'} effort is unavailable, so effort is back to default.`
            : reconciled.modelChanged && reconciled.effort !== null
              ? `Effort stays ${reconciled.effort}.`
              : ''
          setModelChangeNotice(
            [modelMessage, effortMessage].filter(Boolean).join(' '),
          )
        }
        engineTransition.current = null
      },
      (error) => {
        if (!current) return
        setModelOptions(null)
        setIsLoadingModelOptions(false)
        setModelOptionsError(error.message)
        const transition = engineTransition.current
        if (transition?.engine === engine) {
          selectionRef.current = { model: null, effort: null }
          setSelectedModel(null)
          setSelectedEffort(null)
          setModelChangeNotice(
            `Could not load ${engine === 'claude' ? 'Claude Code' : 'Codex'} models, so the model and effort are back to the default.`,
          )
        }
        engineTransition.current = null
      },
    )
    return () => {
      current = false
    }
  }, [active, engine])

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
    setDirectoryQuery('')
    setDirectories([])
    setSelectedDirectory(null)
    setEngine('claude')
    setModelOptions(null)
    setIsLoadingModelOptions(false)
    setModelOptionsError(null)
    setSelectedModel(null)
    setSelectedEffort(null)
    selectionRef.current = { model: null, effort: null }
    setModelChangeNotice(null)
    engineTransition.current = null
    engineUndo.current = null
    setWorktree(false)
    setInitialMessage('')
    setSkillSuggestions([])
    setIsLoadingDirectories(false)
    setError(null)
  }, [])

  const changeEngine = useCallback(
    (nextEngine: SessionEngine) => {
      if (nextEngine === engine) return
      const transition = {
        engine: nextEngine,
        selection: selectionRef.current,
        previousEngine: engine,
      }
      engineTransition.current = transition
      engineUndo.current = transition
      setModelOptions(null)
      setModelChangeNotice(null)
      setEngine(nextEngine)
    },
    [engine],
  )

  const undoEngineChange = useCallback(() => {
    const transition = engineUndo.current
    if (transition === null) return
    engineTransition.current = null
    engineUndo.current = null
    setEngine(transition.previousEngine)
    selectionRef.current = transition.selection
    setSelectedModel(transition.selection.model)
    setSelectedEffort(transition.selection.effort)
    setModelChangeNotice(null)
  }, [])

  const chooseModel = useCallback(
    (model: string | null) => {
      engineUndo.current = null
      setModelChangeNotice(null)
      const selectedEffort = selectionRef.current.effort
      const target =
        model === null
          ? modelOptions === null
            ? undefined
            : defaultEffortModelOption(modelOptions)
          : modelOptions?.models.find((option) => option.id === model)
      const effort =
        selectedEffort !== null &&
        target !== undefined &&
        !target.efforts.includes(selectedEffort)
          ? null
          : selectedEffort
      selectionRef.current = { model, effort }
      setSelectedModel(model)
      setSelectedEffort(effort)
    },
    [modelOptions],
  )

  const chooseEffort = useCallback((effort: string | null) => {
    engineUndo.current = null
    setModelChangeNotice(null)
    selectionRef.current = { ...selectionRef.current, effort }
    setSelectedEffort(effort)
  }, [])

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
      const useSelectedOptions = launchEngine === engine
      const launchModel = useSelectedOptions ? selectedModel : null
      const chosenModel = modelOptions?.models.find(
        (option) => option.id === launchModel,
      )
      const launchEffort = useSelectedOptions
        ? (selectedEffort ??
          (launchModel === null ? null : (chosenModel?.defaultEffort ?? null)))
        : null
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
    [
      engine,
      initialMessage,
      modelOptions,
      selectedDirectory,
      selectedEffort,
      selectedModel,
      worktree,
    ],
  )

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
    modelChangeNotice,
    activeModelOption,
    worktree,
    initialMessage,
    skillSuggestions,
    isLoadingDirectories,
    error,
    setLaunchError: setError,
    setDirectoryQuery,
    chooseDirectory,
    setEngine: changeEngine,
    chooseModel,
    setSelectedEffort: chooseEffort,
    undoEngineChange,
    setWorktree,
    setInitialMessage,
    start,
    reset,
  }
}

export type NewSessionState = ReturnType<typeof useNewSession>
