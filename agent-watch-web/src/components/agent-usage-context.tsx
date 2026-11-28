import { createContext, type ReactNode, useContext } from 'react'

import type { AgentUsage } from '#agent-usage'
import { useAgentUsage } from '#components/use-agent-usage'

type AgentUsageContextValue = {
  usage: AgentUsage | null
  error: boolean
  onUsage: (usage: AgentUsage) => void
  onUsageError: () => void
}

const AgentUsageContext = createContext<AgentUsageContextValue>({
  usage: null,
  error: false,
  onUsage: () => {},
  onUsageError: () => {},
})

export function AgentUsageProvider({ children }: { children: ReactNode }) {
  const value = useAgentUsage()
  return (
    <AgentUsageContext.Provider value={value}>
      {children}
    </AgentUsageContext.Provider>
  )
}

export function useAgentUsageContext() {
  return useContext(AgentUsageContext)
}
