import { createContext, type ReactNode, useContext, useMemo } from 'react'

import type {
  CommandOutputRefresh,
  CommandOutputStart,
} from '#components/command-result-message'

type MarkdownCommandContextValue = {
  sessionId: string
  sessionTitle: string
  onCommandExecuted?: CommandOutputRefresh
  onCommandStarted?: CommandOutputStart
}

const MarkdownCommandContext = createContext<
  MarkdownCommandContextValue | undefined
>(undefined)

export function MarkdownCommandContextProvider({
  children,
  sessionId,
  sessionTitle,
  onCommandExecuted,
  onCommandStarted,
}: MarkdownCommandContextValue & { children: ReactNode }) {
  const value = useMemo(
    () => ({
      sessionId,
      sessionTitle,
      ...(onCommandExecuted === undefined ? {} : { onCommandExecuted }),
      ...(onCommandStarted === undefined ? {} : { onCommandStarted }),
    }),
    [onCommandExecuted, onCommandStarted, sessionId, sessionTitle],
  )

  return (
    <MarkdownCommandContext.Provider value={value}>
      {children}
    </MarkdownCommandContext.Provider>
  )
}

export function useMarkdownCommandContext() {
  return useContext(MarkdownCommandContext)
}
