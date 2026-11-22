import { createContext, type ReactNode, useContext, useMemo } from 'react'

type MarkdownCommandContextValue = {
  sessionId: string
  sessionTitle: string
  onCommandExecuted?: () => void
}

const MarkdownCommandContext = createContext<
  MarkdownCommandContextValue | undefined
>(undefined)

export function MarkdownCommandContextProvider({
  children,
  sessionId,
  sessionTitle,
  onCommandExecuted,
}: MarkdownCommandContextValue & { children: ReactNode }) {
  const value = useMemo(
    () => ({
      sessionId,
      sessionTitle,
      ...(onCommandExecuted === undefined ? {} : { onCommandExecuted }),
    }),
    [onCommandExecuted, sessionId, sessionTitle],
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
