import { createContext } from 'react'

import type { WebUiLinkTarget } from '#components/markdown-session-links'
import type { SessionLinkTarget } from '#types'

export type MarkdownComponentContextValue = {
  sessionsById: ReadonlyMap<string, SessionLinkTarget>
  onSelectSession?: (id: string) => void
  webUis: readonly WebUiLinkTarget[]
  onOpenWebUi?: (webUi: WebUiLinkTarget, url: string) => void
  commandContext?: {
    messageText: string
    sessionId: string
    onCommandExecuted?: () => void
  }
}

export const MarkdownComponentContext =
  createContext<MarkdownComponentContextValue>({
    sessionsById: new Map(),
    webUis: [],
  })
