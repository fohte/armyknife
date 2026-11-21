import { createContext } from 'react'

import type { WebUiLinkTarget } from '#components/markdown-session-links'
import type { SessionLinkTarget } from '#types'

export type MarkdownComponentContextValue = {
  sessionsById: ReadonlyMap<string, SessionLinkTarget>
  onSelectSession?: (id: string) => void
  copyableCodeBlocks: boolean
  webUis: readonly WebUiLinkTarget[]
  onOpenWebUi?: (webUi: WebUiLinkTarget, url: string) => void
  mobile?: boolean
  commandContext?: {
    messageText: string
    sessionId: string
    onCommandExecuted?: () => void
  }
}

export const MarkdownComponentContext =
  createContext<MarkdownComponentContextValue>({
    sessionsById: new Map(),
    copyableCodeBlocks: false,
    webUis: [],
  })
