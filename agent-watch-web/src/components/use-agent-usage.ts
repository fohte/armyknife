import { useCallback, useState } from 'react'

import type { AgentUsage } from '#agent-usage'

export function useAgentUsage() {
  const [usage, setUsage] = useState<AgentUsage | null>(null)
  const [error, setError] = useState(false)

  const onUsage = useCallback((nextUsage: AgentUsage) => {
    setUsage(nextUsage)
    setError(false)
  }, [])
  const onUsageError = useCallback(() => {
    setError(true)
  }, [])

  return { usage, error, onUsage, onUsageError }
}
