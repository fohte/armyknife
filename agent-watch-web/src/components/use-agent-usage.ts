import { useEffect, useState } from 'react'

import { type AgentUsage, requestAgentUsage } from '#agent-usage'

const USAGE_REFRESH_INTERVAL = 60_000

export function useAgentUsage() {
  const [usage, setUsage] = useState<AgentUsage | null>(null)
  const [error, setError] = useState(false)

  useEffect(() => {
    let active = true
    let timeoutId: number | null = null

    const refresh = async () => {
      const result = await requestAgentUsage()
      if (!active) return
      result.match(
        (nextUsage) => {
          setUsage(nextUsage)
          setError(false)
        },
        () => {
          setError(true)
        },
      )
      timeoutId = window.setTimeout(() => {
        void refresh()
      }, USAGE_REFRESH_INTERVAL)
    }

    void refresh()
    return () => {
      active = false
      if (timeoutId !== null) window.clearTimeout(timeoutId)
    }
  }, [])

  return { usage, error }
}
