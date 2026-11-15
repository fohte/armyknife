import { createContext, type ReactNode, useContext, useMemo } from 'react'

import type { SessionLinkTarget } from '#types'

type MarkdownSessionLinksValue = {
  sessions: readonly SessionLinkTarget[]
  onSelectSession?: (id: string) => void
}

const MarkdownSessionLinksContext = createContext<MarkdownSessionLinksValue>({
  sessions: [],
})

export function MarkdownSessionLinksProvider({
  children,
  sessions,
  onSelectSession,
}: MarkdownSessionLinksValue & { children: ReactNode }) {
  const value = useMemo(
    () => ({
      sessions,
      ...(onSelectSession === undefined ? {} : { onSelectSession }),
    }),
    [onSelectSession, sessions],
  )

  return (
    <MarkdownSessionLinksContext.Provider value={value}>
      {children}
    </MarkdownSessionLinksContext.Provider>
  )
}

export function useMarkdownSessionLinks() {
  return useContext(MarkdownSessionLinksContext)
}
