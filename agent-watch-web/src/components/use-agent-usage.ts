import { useEffect, useState } from 'react'

import { type AgentUsage, requestAgentUsage } from '#agent-usage'
import { isPageVisible } from '#session-visibility'

const USAGE_REFRESH_INTERVAL = 60_000

export function useAgentUsage() {
  const [usage, setUsage] = useState<AgentUsage | null>(null)
  const [error, setError] = useState(false)

  useEffect(() => {
    let active = true
    let visible = isPageVisible(document)
    let timeoutId: number | null = null
    let requestPending = false
    let refreshOnResume = false
    const canRefresh = () => active && visible

    const refresh = async () => {
      if (!canRefresh()) return
      if (requestPending) {
        refreshOnResume = true
        return
      }
      requestPending = true
      const result = await requestAgentUsage()
      requestPending = false
      if (!canRefresh()) return
      result.match(
        (nextUsage) => {
          setUsage(nextUsage)
          setError(false)
        },
        () => {
          setError(true)
        },
      )
      if (refreshOnResume) {
        refreshOnResume = false
        void refresh()
        return
      }
      timeoutId = window.setTimeout(() => {
        void refresh()
      }, USAGE_REFRESH_INTERVAL)
    }

    const handleVisibilityChange = () => {
      const wasVisible = visible
      visible = isPageVisible(document)
      if (!visible) {
        if (timeoutId !== null) window.clearTimeout(timeoutId)
        timeoutId = null
        if (requestPending) refreshOnResume = true
      } else if (!wasVisible) {
        if (requestPending) {
          refreshOnResume = true
        } else {
          refreshOnResume = false
          void refresh()
        }
      }
    }

    document.addEventListener('visibilitychange', handleVisibilityChange)
    if (visible) void refresh()
    return () => {
      active = false
      document.removeEventListener('visibilitychange', handleVisibilityChange)
      if (timeoutId !== null) window.clearTimeout(timeoutId)
    }
  }, [])

  return { usage, error }
}
