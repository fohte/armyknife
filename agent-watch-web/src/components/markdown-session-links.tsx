import { createContext, type ReactNode, useContext, useMemo } from 'react'

import type { SessionLinkTarget } from '#types'

type MarkdownSessionLinksValue = {
  sessions: readonly SessionLinkTarget[]
  onSelectSession?: (id: string) => void
  webUis?: readonly WebUiLinkTarget[]
  onOpenWebUi?: (webUi: WebUiLinkTarget, url: string) => void
}

export type WebUiLinkTarget = {
  name: string
  url: string
  exposedUrl?: string | null
}

const MarkdownSessionLinksContext = createContext<MarkdownSessionLinksValue>({
  sessions: [],
})

export function MarkdownSessionLinksProvider({
  children,
  sessions,
  onSelectSession,
  webUis = [],
  onOpenWebUi,
}: MarkdownSessionLinksValue & { children: ReactNode }) {
  const value = useMemo(
    () => ({
      sessions,
      webUis,
      ...(onSelectSession === undefined ? {} : { onSelectSession }),
      ...(onOpenWebUi === undefined ? {} : { onOpenWebUi }),
    }),
    [onOpenWebUi, onSelectSession, sessions, webUis],
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
