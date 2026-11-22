import { createContext } from 'react'

import type {
  CommandOutputRefresh,
  CommandOutputStart,
} from '#components/command-result-message'
import type { WebUiLinkTarget } from '#components/markdown-session-links'
import type { SessionLinkTarget } from '#types'

export type MarkdownComponentContextValue = {
  sessionsById: ReadonlyMap<string, SessionLinkTarget>
  onSelectSession?: (id: string) => void
  webUis: readonly WebUiLinkTarget[]
  onOpenWebUi?: (webUi: WebUiLinkTarget, url: string) => void
  mobile?: boolean
  commandContext?: {
    messageText: string
    sessionId: string
    sessionTitle: string
    onCommandExecuted?: CommandOutputRefresh
    onCommandStarted?: CommandOutputStart
  }
}

export const MarkdownComponentContext =
  createContext<MarkdownComponentContextValue>({
    sessionsById: new Map(),
    webUis: [],
  })
