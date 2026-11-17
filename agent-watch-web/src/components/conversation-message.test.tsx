import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { renderConversationMessage } from '#components/conversation-message'

it('renders narration as muted conversation text', () => {
  expect(
    renderToStaticMarkup(
      renderConversationMessage({
        actions: null,
        actionsReady: true,
        isMobile: false,
        selected: false,
        decisionPosition: undefined,
        onSelectDecision: () => undefined,
        message: {
          type: 'narration',
          text: 'I will inspect the fictional request before answering.',
        },
        pendingAction: null,
        openInTerminalStatus: null,
        onAction: () => Promise.resolve(true),
        onOpenInTerminal: () => undefined,
        onSelectSession: () => undefined,
        sessionEngine: 'claude',
        sessions: [],
      }),
    ),
  ).toBe(
    '<article class="chat-narration-message"><div class="markdown-message"><p>I will inspect the fictional request before answering.</p></div></article>',
  )
})
