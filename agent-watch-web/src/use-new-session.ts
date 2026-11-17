import { useCallback, useEffect, useRef, useState } from 'react'

import {
  getSessionDirectories,
  getSessionSkillSuggestions,
  launchSession,
  type SessionDirectory,
  type SessionEngine,
} from '#new-session-api'
import type { SkillSuggestion } from '#types'

export function useNewSession(active: boolean) {
  const [directoryQuery, setDirectoryQuery] = useState('')
  const [directories, setDirectories] = useState<SessionDirectory[]>([])
  const [selectedDirectory, setSelectedDirectory] =
    useState<SessionDirectory | null>(null)
  const [engine, setEngine] = useState<SessionEngine>('claude')
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
    setWorktree(false)
    setInitialMessage('')
    setSkillSuggestions([])
    setIsLoadingDirectories(false)
    setIsStarting(false)
    setError(null)
  }, [])

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
      const sequence = launchSequence.current + 1
      launchSequence.current = sequence
      setError(null)
      setIsStarting(true)
      const result = await launchSession({
        directory: selectedDirectory.path,
        engine: engineOverride ?? engine,
        worktree,
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
    [engine, initialMessage, isStarting, selectedDirectory, worktree],
  )

  return {
    directoryQuery,
    directories,
    selectedDirectory,
    engine,
    worktree,
    initialMessage,
    skillSuggestions,
    isLoadingDirectories,
    isStarting,
    error,
    setDirectoryQuery,
    chooseDirectory,
    setEngine,
    setWorktree,
    setInitialMessage,
    start,
    reset,
  }
}

export type NewSessionState = ReturnType<typeof useNewSession>
