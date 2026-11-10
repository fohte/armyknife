import { useCallback, useEffect, useRef, useState } from 'react'

import type { SessionView } from '#session-navigation'

export type SessionHistoryState = {
  selectedId: string | null
  view: SessionView
  conversationOpen: boolean
}

export type HistoryMode = 'push' | 'replace'

const sessionHistoryKey = 'agentWatchSessionHistory'

type StateUpdate =
  SessionHistoryState | ((current: SessionHistoryState) => SessionHistoryState)

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return isRecord(value) ? value : null
}

function isSessionHistoryState(value: unknown): value is SessionHistoryState {
  const record = asRecord(value)
  return (
    record !== null &&
    (typeof record['selectedId'] === 'string' ||
      record['selectedId'] === null) &&
    (record['view'] === 'inbox' || record['view'] === 'board') &&
    typeof record['conversationOpen'] === 'boolean'
  )
}

function readPreviousNavigation(value: unknown): SessionHistoryState | null {
  const historyState = asRecord(value)
  const entry = asRecord(historyState?.[sessionHistoryKey])
  const previousNavigation = entry?.['previousNavigation']
  return isSessionHistoryState(previousNavigation) ? previousNavigation : null
}

function readSessionHistoryState(url: URL): SessionHistoryState {
  const view = url.searchParams.get('view')
  return {
    selectedId: url.searchParams.get('session'),
    view: view === 'board' ? 'board' : 'inbox',
    conversationOpen: url.searchParams.get('screen') === 'conversation',
  }
}

function sessionHistoryUrl(state: SessionHistoryState, currentUrl: URL): URL {
  const url = new URL(currentUrl)
  url.searchParams.delete('session')
  url.searchParams.delete('view')
  url.searchParams.delete('screen')
  if (state.selectedId !== null) {
    url.searchParams.set('session', state.selectedId)
  }
  if (state.view === 'board') {
    url.searchParams.set('view', 'board')
  }
  if (state.conversationOpen) {
    url.searchParams.set('screen', 'conversation')
  }
  return url
}

function writeSessionHistory(
  state: SessionHistoryState,
  mode: HistoryMode,
  previousNavigation: SessionHistoryState,
) {
  const url = sessionHistoryUrl(state, new URL(window.location.href))
  const existingState = asRecord(window.history.state)
  const nextHistoryState = existingState === null ? {} : { ...existingState }
  nextHistoryState[sessionHistoryKey] = {
    previousNavigation:
      mode === 'push'
        ? previousNavigation
        : readPreviousNavigation(window.history.state),
  }
  if (mode === 'push') {
    window.history.pushState(nextHistoryState, '', url)
  } else {
    window.history.replaceState(nextHistoryState, '', url)
  }
}

export function useSessionHistory() {
  const stateRef = useRef(
    readSessionHistoryState(new URL(window.location.href)),
  )
  const [state, setState] = useState(stateRef.current)

  useEffect(() => {
    const onPopState = () => {
      const nextState = readSessionHistoryState(new URL(window.location.href))
      stateRef.current = nextState
      setState(nextState)
    }
    window.addEventListener('popstate', onPopState)
    return () => {
      window.removeEventListener('popstate', onPopState)
    }
  }, [])

  const navigate = useCallback(
    (update: StateUpdate, mode: HistoryMode = 'push') => {
      const nextState =
        typeof update === 'function' ? update(stateRef.current) : update
      const previousState = stateRef.current
      stateRef.current = nextState
      writeSessionHistory(nextState, mode, previousState)
      setState(nextState)
    },
    [],
  )

  const backToInbox = useCallback(() => {
    const currentState = stateRef.current
    const previousNavigation = readPreviousNavigation(window.history.state)
    if (
      currentState.selectedId !== null &&
      currentState.view === 'inbox' &&
      previousNavigation?.selectedId === currentState.selectedId &&
      previousNavigation.view === 'inbox' &&
      !previousNavigation.conversationOpen
    ) {
      window.history.back()
      return
    }
    navigate((current) => ({ ...current, conversationOpen: false }), 'replace')
  }, [navigate])

  return { ...state, backToInbox, navigate }
}
