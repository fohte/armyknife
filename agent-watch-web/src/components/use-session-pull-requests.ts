import { useEffect, useState } from 'react'

import { isSessionPullRequestListResponse } from '#components/conversation-pane-response'
import { requestJson } from '#components/conversation-requests'
import type { CachedPullRequestList } from '#components/session-pull-request-cache'
import {
  beginPullRequestListRequest,
  getCachedPullRequestList,
  isCurrentPullRequestListRequest,
  setCachedPullRequestList,
} from '#components/session-pull-request-cache'
import { isPageVisible, subscribeToPageVisibility } from '#session-visibility'
import type { Session, SessionPullRequest } from '#types'

const PULL_REQUEST_REFRESH_INTERVAL = 30_000

type PullRequestListState = {
  cacheKey: string | null
  pullRequests: SessionPullRequest[]
  loading: boolean
  error: string | null
}

type UseSessionPullRequestsOptions = {
  session: Session
  enabled: boolean
}

function isPullRequestListStale(
  cached: CachedPullRequestList | undefined,
): boolean {
  return (
    cached === undefined ||
    Date.now() - cached.fetchedAt >= PULL_REQUEST_REFRESH_INTERVAL
  )
}

export function useSessionPullRequests({
  session,
  enabled,
}: UseSessionPullRequestsOptions) {
  const cacheKey = JSON.stringify([session.id, session.task?.id ?? null])
  const [state, setState] = useState<PullRequestListState>({
    cacheKey: null,
    pullRequests: [],
    loading: false,
    error: null,
  })
  useEffect(() => {
    if (!enabled) return

    const visibilityTarget =
      typeof document === 'undefined' ? undefined : document
    let current = true
    let visible = isPageVisible(visibilityTarget)
    let inFlight = false
    let refreshQueued = false
    let refreshTimeout: number | null = null

    const stopScheduledRefresh = () => {
      if (refreshTimeout === null) return
      window.clearTimeout(refreshTimeout)
      refreshTimeout = null
    }
    const scheduleRefresh = () => {
      stopScheduledRefresh()
      if (!current || !visible) return
      const cached = getCachedPullRequestList(cacheKey)
      const dueAt =
        (cached?.lastAttemptAt ?? Date.now()) + PULL_REQUEST_REFRESH_INTERVAL
      refreshTimeout = window.setTimeout(
        () => {
          refresh()
        },
        Math.max(0, dueAt - Date.now()),
      )
    }
    const refresh = () => {
      if (!current || !visible) return
      if (inFlight) {
        refreshQueued = true
        return
      }
      inFlight = true
      const cached = getCachedPullRequestList(cacheKey)
      setState((previous) => {
        const hasCurrentList = previous.cacheKey === cacheKey
        return {
          cacheKey,
          pullRequests: hasCurrentList
            ? previous.pullRequests
            : (cached?.pullRequests ?? []),
          loading: !hasCurrentList && cached === undefined,
          error: hasCurrentList ? previous.error : (cached?.error ?? null),
        }
      })
      const requestedAt = Date.now()
      const generation = beginPullRequestListRequest(cacheKey)
      void requestJson(
        `/api/sessions/${encodeURIComponent(session.id)}/pull-requests`,
        isSessionPullRequestListResponse,
      )
        .match(
          (response) => {
            if (!isCurrentPullRequestListRequest(cacheKey, generation)) return
            const fetchedAt = Date.now()
            setCachedPullRequestList(cacheKey, {
              pullRequests: response.pullRequests,
              error: response.error,
              fetchedAt,
              lastAttemptAt: requestedAt,
            })
            if (current) {
              setState({
                cacheKey,
                pullRequests: response.pullRequests,
                loading: false,
                error: response.error,
              })
            }
          },
          (error) => {
            if (!isCurrentPullRequestListRequest(cacheKey, generation)) return
            const previous = getCachedPullRequestList(cacheKey)
            setCachedPullRequestList(cacheKey, {
              pullRequests: previous?.pullRequests ?? [],
              error: error.message,
              fetchedAt: previous?.fetchedAt ?? requestedAt,
              lastAttemptAt: requestedAt,
            })
            if (current) {
              setState((previous) => ({
                cacheKey,
                pullRequests:
                  previous.cacheKey === cacheKey ? previous.pullRequests : [],
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
            return
          }
          scheduleRefresh()
        })
    }
    const onVisibilityChange = (nextVisible: boolean) => {
      if (!current || nextVisible === visible) return
      visible = nextVisible
      if (!visible) {
        refreshQueued = false
        stopScheduledRefresh()
        return
      }
      const cached = getCachedPullRequestList(cacheKey)
      if (isPullRequestListStale(cached)) {
        refresh()
      } else {
        scheduleRefresh()
      }
    }
    const unsubscribeVisibility = subscribeToPageVisibility(
      visibilityTarget,
      onVisibilityChange,
    )

    const cached = getCachedPullRequestList(cacheKey)
    if (cached !== undefined) {
      setState({
        cacheKey,
        pullRequests: cached.pullRequests,
        loading: false,
        error: cached.error,
      })
    }
    if (visible) {
      if (isPullRequestListStale(cached)) {
        refresh()
      } else {
        scheduleRefresh()
      }
    }

    return () => {
      current = false
      stopScheduledRefresh()
      unsubscribeVisibility()
    }
  }, [enabled, session.id, cacheKey])

  if (state.cacheKey === cacheKey) {
    return state
  }
  const cached = getCachedPullRequestList(cacheKey)
  if (cached !== undefined) {
    return {
      cacheKey,
      pullRequests: cached.pullRequests,
      loading: false,
      error: cached.error,
    }
  }
  return {
    cacheKey,
    pullRequests: [],
    loading: enabled,
    error: null,
  }
}
