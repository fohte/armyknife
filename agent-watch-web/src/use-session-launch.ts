import { ResultAsync, type ResultAsync as ResultAsyncType } from 'neverthrow'
import { useCallback, useEffect, useRef, useState } from 'react'

import { useAsyncActionStarter } from '#async-action-feedback'
import { launchSession, type LaunchSessionRequest } from '#new-session-api'
import { findLaunchedSession } from '#new-session-navigation'
import type { HistoryMode, SessionHistoryState } from '#session-history'
import type { Session } from '#types'
import type { useNewSession } from '#use-new-session'

type PendingLaunch = {
  key: number
  request: LaunchSessionRequest
  startedAt: number
  result: ResultAsyncType<string, Error>
}

type UseSessionLaunchOptions = {
  sessions: readonly Session[]
  newSessionMode: 'form' | 'palette' | null
  view: 'inbox' | 'board'
  newSession: ReturnType<typeof useNewSession>
  setCommandPaletteOpen: (isOpen: boolean) => void
  openSessionInInbox: (id: string, mode?: 'push' | 'replace') => void
  openSessionFromNavigation: (id: string) => void
  navigate: (
    update: (current: SessionHistoryState) => SessionHistoryState,
    mode?: HistoryMode,
  ) => void
}

function toError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('Session launch failed', { cause: error })
}

export function useSessionLaunch({
  sessions,
  newSessionMode,
  view,
  newSession,
  setCommandPaletteOpen,
  openSessionInInbox,
  openSessionFromNavigation,
  navigate,
}: UseSessionLaunchOptions) {
  const [pendingLaunch, setPendingLaunch] = useState<PendingLaunch | null>(null)
  const pendingLaunchRef = useRef<PendingLaunch | null>(null)
  const sessionsRef = useRef(sessions)
  sessionsRef.current = sessions
  const sequence = useRef(0)
  const waiters = useRef(new Map<string, (sessionId: string) => void>())
  const startAsyncAction = useAsyncActionStarter((feedbackError) => {
    newSession.setLaunchError(feedbackError.message)
  })

  const updatePendingLaunch = useCallback((next: PendingLaunch | null) => {
    pendingLaunchRef.current = next
    setPendingLaunch(next)
  }, [])

  const launch = useCallback(
    (request: LaunchSessionRequest) =>
      launchSession(request).andThen(({ launchId }) =>
        ResultAsync.fromPromise(
          new Promise<string>((resolve) => {
            const launchedSession = findLaunchedSession(
              sessionsRef.current,
              launchId,
            )
            if (launchedSession !== undefined) {
              resolve(launchedSession.id)
              return
            }
            waiters.current.set(launchId, resolve)
          }),
          toError,
        ),
      ),
    [],
  )

  useEffect(() => {
    for (const [launchId, resolve] of waiters.current) {
      const launchedSession = findLaunchedSession(sessions, launchId)
      if (launchedSession === undefined) continue
      waiters.current.delete(launchId)
      resolve(launchedSession.id)
    }
  }, [sessions])

  const start = useCallback(
    (request: LaunchSessionRequest) => {
      if (pendingLaunchRef.current !== null) return
      sequence.current += 1
      const key = sequence.current
      const result = launch(request)
      const next = { key, request, startedAt: Date.now(), result }
      updatePendingLaunch(next)
      void result.match(
        (sessionId) => {
          if (pendingLaunchRef.current?.key !== key) return
          updatePendingLaunch(null)
          newSession.setLaunchError(null)
          setCommandPaletteOpen(false)
          openSessionInInbox(sessionId, 'replace')
        },
        (error) => {
          if (pendingLaunchRef.current?.key !== key) return
          updatePendingLaunch(null)
          newSession.setLaunchError(error.message)
        },
      )
    },
    [
      launch,
      newSession.setLaunchError,
      openSessionInInbox,
      setCommandPaletteOpen,
      updatePendingLaunch,
    ],
  )

  const handOff = useCallback(() => {
    const current = pendingLaunchRef.current
    if (current === null) return false
    let useExistingResult = true
    let launchedSessionId: string | null = null
    const run = async () => {
      const result = useExistingResult
        ? current.result
        : launch(current.request)
      useExistingResult = false
      return result.match(
        (sessionId) => {
          launchedSessionId = sessionId
          return { status: 'launched' as const }
        },
        (error) => ({ status: 'launchFailed' as const, error: error.message }),
      )
    }
    startAsyncAction({
      kind: 'launch',
      sessionId: `launch-${String(current.key)}`,
      sessionTitle: current.request.directory,
      initialMessage: current.request.initialMessage,
      repo: current.request.directory,
      createdAt: current.startedAt,
      run,
      onOpenSession: () => {
        if (launchedSessionId !== null) {
          openSessionFromNavigation(launchedSessionId)
        }
      },
    })
    updatePendingLaunch(null)
    newSession.reset()
    return true
  }, [
    launch,
    newSession.reset,
    openSessionFromNavigation,
    startAsyncAction,
    updatePendingLaunch,
  ])

  useEffect(() => {
    if (pendingLaunch === null) return
    const shouldKeepLaunchOnScreen =
      newSessionMode === 'palette' ||
      (newSessionMode === 'form' && view === 'inbox')
    if (shouldKeepLaunchOnScreen) return
    handOff()
    if (newSessionMode === 'form') {
      navigate((current) => ({ ...current, newSessionMode: null }), 'replace')
    }
  }, [handOff, navigate, newSessionMode, pendingLaunch, view])

  return { pendingLaunch, start, handOff }
}
