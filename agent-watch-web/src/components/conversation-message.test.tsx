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

it('renders standalone thinking as a collapsible thought group', () => {
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
          type: 'thinking',
          text: 'Review the fictional request.',
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
    '<details class="chat-tool-summary" data-has-operations="true"><summary>Thought ×1</summary><ul class="chat-tool-operations"><li class="chat-tool-thought"><details><summary><span class="chat-tool-name">Thought</span><span class="chat-tool-thought-preview">Review the fictional request.</span></summary><p class="chat-tool-thought-full">Review the fictional request.</p></details></li></ul></details>',
  )
})
