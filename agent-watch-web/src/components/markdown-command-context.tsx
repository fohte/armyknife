import { createContext, type ReactNode, useContext, useMemo } from 'react'

type MarkdownCommandContextValue = {
  sessionId: string
  onCommandExecuted?: () => void
}

const MarkdownCommandContext = createContext<
  MarkdownCommandContextValue | undefined
>(undefined)

export function MarkdownCommandContextProvider({
  children,
  sessionId,
  onCommandExecuted,
}: MarkdownCommandContextValue & { children: ReactNode }) {
  const value = useMemo(
    () => ({
      sessionId,
      ...(onCommandExecuted === undefined ? {} : { onCommandExecuted }),
    }),
    [onCommandExecuted, sessionId],
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
