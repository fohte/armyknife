import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { renderConversationMessage } from '#components/conversation-message'
import type { ChatItem } from '#types'

const localImagePath = '/Users/guest/screenshot.png'
const servedImageSource =
  '/api/sessions/fixture-session/answer-images/fixture-image'

function renderConversationCardImageSource(message: ChatItem) {
  const markup = renderToStaticMarkup(
    renderConversationMessage({
      actions: null,
      actionsReady: true,
      isMobile: false,
      selected: false,
      decisionPosition: undefined,
      onSelectDecision: () => undefined,
      message,
      imageSources: { [localImagePath]: servedImageSource },
      pendingAction: null,
      openInTerminalStatus: null,
      onAction: () => Promise.resolve(true),
      onOpenInTerminal: () => undefined,
      onSelectSession: () => undefined,
      sessionEngine: 'claude',
      sessionId: 'fixture-session',
      sessions: [],
    }),
  )
  return markup.match(/<img[^>]*\bsrc="([^"]+)"/)?.[1] ?? null
}

const cardImageCases: { name: string; message: ChatItem }[] = [
  {
    name: 'to-do detail',
    message: {
      type: 'todo',
      seq: 1,
      title: 'Review the draft',
      detail: `![](${localImagePath})`,
      completed: false,
      resolution: null,
      comment: null,
    },
  },
  {
    name: 'proposal summary',
    message: {
      type: 'proposal',
      seq: 2,
      title: 'Choose a layout',
      summary: `![](${localImagePath})`,
      options: [],
      answer: null,
      resolved: false,
      resolution: null,
      comment: null,
    },
  },
]

it.each(cardImageCases)(
  'uses the served image source in a $name card',
  ({ message }) => {
    expect(renderConversationCardImageSource(message)).toBe(servedImageSource)
  },
)

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
    '<article class="chat-assistant-message chat-narration-message"><div class="markdown-message"><p>I will inspect the fictional request before answering.</p></div></article>',
  )
})

it('renders narration links and code blocks with assistant message controls', () => {
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
          text: '[the guide](https://example.com/guide)\n\n```sh\n! printf sample\n```',
        },
        pendingAction: null,
        openInTerminalStatus: null,
        onAction: () => Promise.resolve(true),
        onOpenInTerminal: () => undefined,
        onSelectSession: () => undefined,
        sessionEngine: 'claude',
        sessionId: 'fictional-session',
        sessions: [],
      }),
    ),
  ).toBe(
    [
      '<article class="chat-assistant-message chat-narration-message"><div class="markdown-message"><p><a href="https://example.com/guide" target="_blank" rel="noopener noreferrer">the guide</a></p>',
      '<div class="markdown-code-block"><div class="syntax-code-block-container"><div class="syntax-code-block-toolbar"><span class="syntax-code-block-language">sh</span><button class="syntax-code-block-copy-button" type="button" aria-label="Copy code" title="Copy code">Copy</button></div><pre class="syntax-code-block" data-language="sh"><code class="language-sh">! printf sample',
      '</code></pre></div><span class="bang-command-actions"><span class="bang-command-control"><button aria-label="Run command 1" class="bang-command-button" type="button">Run command 1</button></span></span></div></div></article>',
    ].join(String.fromCharCode(10)),
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
    '<details class="chat-tool-summary" data-has-operations="true"><summary>Thought ×1</summary><ul class="chat-tool-operations"><li class="chat-tool-thought"><details><summary><span class="chat-tool-name">Thought</span><span class="chat-tool-thought-preview">Review the fictional request.</span><span class="chat-tool-thought-length">29 chars</span></summary><p class="chat-tool-thought-full">Review the fictional request.</p></details></li></ul></details>',
  )
})
