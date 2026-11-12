import { useCallback, useEffect, useState } from 'react'

import { isSessionPullRequestListResponse } from '#components/conversation-pane-response'
import { requestJson } from '#components/conversation-requests'
import { isPageVisible, subscribeToPageVisibility } from '#session-visibility'
import type { Session, SessionPullRequest } from '#types'

const PULL_REQUEST_REFRESH_INTERVAL = 30_000

type PullRequestListState = {
  sessionId: string | null
  pullRequests: SessionPullRequest[]
  loading: boolean
  error: string | null
}

type UseSessionPullRequestsOptions = {
  session: Session
  enabled: boolean
}

export function useSessionPullRequests({
  session,
  enabled,
}: UseSessionPullRequestsOptions) {
  const [state, setState] = useState<PullRequestListState>({
    sessionId: null,
    pullRequests: [],
    loading: false,
    error: null,
  })
  const [refreshVersion, setRefreshVersion] = useState(0)
  const refreshNow = useCallback(() => {
    setRefreshVersion((version) => version + 1)
  }, [])

  useEffect(() => {
    if (!enabled) return

    const visibilityTarget =
      typeof document === 'undefined' ? undefined : document
    let current = true
    let visible = isPageVisible(visibilityTarget)
    let inFlight = false
    let refreshQueued = false
    let interval: number | null = null

    const stopInterval = () => {
      if (interval === null) return
      window.clearInterval(interval)
      interval = null
    }
    const startInterval = () => {
      if (interval !== null) return
      interval = window.setInterval(() => {
        refresh()
      }, PULL_REQUEST_REFRESH_INTERVAL)
    }
    const refresh = () => {
      if (!current || !visible) return
      if (inFlight) {
        refreshQueued = true
        return
      }
      inFlight = true
      setState((previous) => ({
        sessionId: session.id,
        pullRequests:
          previous.sessionId === session.id ? previous.pullRequests : [],
        loading: true,
        error: null,
      }))
      void requestJson(
        `/api/sessions/${encodeURIComponent(session.id)}/pull-requests`,
        isSessionPullRequestListResponse,
      )
        .match(
          (response) => {
            if (current) {
              setState({
                sessionId: session.id,
                pullRequests: response.pullRequests,
                loading: false,
                error: response.error,
              })
            }
          },
          (error) => {
            if (current) {
              setState((previous) => ({
                sessionId: session.id,
                pullRequests:
                  previous.sessionId === session.id
                    ? previous.pullRequests
                    : [],
                loading: false,
                error: error.message,
              }))
            }
          },
        )
        .then(() => {
          inFlight = false
          if (current && visible && refreshQueued) {
            refreshQueued = false
            refresh()
          }
        })
    }
    const onVisibilityChange = (nextVisible: boolean) => {
      if (!current || nextVisible === visible) return
      visible = nextVisible
      if (!visible) {
        refreshQueued = false
        stopInterval()
        return
      }
      refresh()
      startInterval()
    }
    const unsubscribeVisibility = subscribeToPageVisibility(
      visibilityTarget,
      onVisibilityChange,
    )

    if (visible) {
      refresh()
      startInterval()
    }

    return () => {
      current = false
      stopInterval()
      unsubscribeVisibility()
    }
  }, [enabled, session.id, session.task?.id, refreshVersion])

  if (state.sessionId === session.id) {
    return { ...state, refresh: refreshNow }
  }
  return {
    sessionId: session.id,
    pullRequests: [],
    loading: enabled,
    error: null,
    refresh: refreshNow,
  }
}
