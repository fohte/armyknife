import { useCallback, useEffect, useRef, useState } from 'react'

import type { SessionView } from '#session-navigation'

export type SessionHistoryState = {
  selectedId: string | null
  view: SessionView
  conversationOpen: boolean
}

export type HistoryMode = 'push' | 'replace'

type StateUpdate =
  SessionHistoryState | ((current: SessionHistoryState) => SessionHistoryState)

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

function writeSessionHistory(state: SessionHistoryState, mode: HistoryMode) {
  const url = sessionHistoryUrl(state, new URL(window.location.href))
  if (mode === 'push') {
    window.history.pushState(null, '', url)
  } else {
    window.history.replaceState(null, '', url)
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
      stateRef.current = nextState
      writeSessionHistory(nextState, mode)
      setState(nextState)
    },
    [],
  )

  return { ...state, navigate }
}
