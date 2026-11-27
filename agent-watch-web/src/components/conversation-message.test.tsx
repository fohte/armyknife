import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { renderConversationMessage } from '#components/conversation-message'
import { ConversationMessageRow } from '#components/conversation-message-row'
import type { ChatItem } from '#types'

const localImagePath = '/Users/guest/screenshot.png'
const servedImageSource =
  '/api/sessions/fixture-session/answer-images/fixture-image'

function renderConversationCardMarkup(
  message: ChatItem,
  includeImageSources = true,
) {
  const markup = renderToStaticMarkup(
    <ConversationMessageRow
      actions={null}
      actionsReady={true}
      isMobile={false}
      selected={false}
      decisionPosition={undefined}
      onSelectDecision={() => undefined}
      message={message}
      {...(includeImageSources
        ? { imageSources: { [localImagePath]: servedImageSource } }
        : {})}
      pendingAction={null}
      openInTerminalStatus={null}
      onAction={() => Promise.resolve(true)}
      onOpenInTerminal={() => undefined}
      onSelectSession={() => undefined}
      sessionEngine="claude"
      sessionId="fixture-session"
      sessions={[]}
    />,
  )
  return markup.replace(/_R_[\da-z]+_/g, 'GENERATED_ID')
}

const mappedImageMarkup =
  '<button class="image-lightbox-trigger" type="button" aria-label="Enlarge image"><img class="chat-user-image" src="/api/sessions/fixture-session/answer-images/fixture-image" alt="" loading="lazy" decoding="async"/></button>'
const unavailableImageMarkup =
  '<span class="markdown-image-unavailable">表示できません: /Users/guest/screenshot.png</span>'

function expectedTodoCardMarkup(imageMarkup: string) {
  return [
    '<article class="chat-todo-card" aria-label="User task" aria-busy="false" data-resolution="pending"><header class="chat-todo-heading"><h2>To-do · pending</h2>',
    '<button type="button" tabindex="0" data-slot="button" aria-label="Fold this to-do" title="Fold" aria-expanded="true" class="group/button inline-flex shrink-0 items-center justify-center outline-none select-none focus-visible:ring-3 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50 aria-invalid:ring-3 aria-invalid:ring-destructive/40 [&amp;_svg]:pointer-events-none [&amp;_svg]:shrink-0 [&amp;_svg:not([class*=&#x27;size-&#x27;])]:size-4 text-muted-foreground hover:text-destructive chat-todo-fold-button cursor-pointer"><svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="lucide lucide-chevron-up" aria-hidden="true"><path d="m18 15-6-6-6 6"></path></svg></button></header><h3>Review the draft</h3>',
    `<div class="collapsible-text chat-todo-detail"><div id="GENERATED_ID" class="collapsible-text__content" data-expanded="false"><div class="markdown-message"><p>${imageMarkup}</p></div></div></div>`,
    '<label class="chat-todo-comment-label" for="GENERATED_ID">Comment (optional)</label><p class="conversation-error" role="status">Browser storage is unavailable. Drafts may not survive a page reload.</p><div class="chat-card-comment-drop-zone"><textarea id="GENERATED_ID" class="chat-todo-comment" rows="2"></textarea></div>',
    '<footer class="chat-todo-footer"><button type="button" tabindex="0" data-slot="button" class="group/button inline-flex shrink-0 items-center justify-center outline-none select-none focus-visible:ring-3 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50 aria-invalid:ring-3 aria-invalid:ring-destructive/40 [&amp;_svg]:pointer-events-none [&amp;_svg]:shrink-0 [&amp;_svg:not([class*=&#x27;size-&#x27;])]:size-4 text-muted-foreground hover:text-destructive aw-button aw-button--primary">Mark done</button><button type="button" tabindex="0" data-slot="button" class="group/button inline-flex shrink-0 items-center justify-center outline-none select-none focus-visible:ring-3 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50 aria-invalid:ring-3 aria-invalid:ring-destructive/40 [&amp;_svg]:pointer-events-none [&amp;_svg]:shrink-0 [&amp;_svg:not([class*=&#x27;size-&#x27;])]:size-4 text-muted-foreground hover:text-destructive aw-button chat-todo-reject">Reject</button></footer></article>',
  ].join('')
}

const todoMessage: ChatItem = {
  type: 'todo',
  seq: 1,
  title: 'Review the draft',
  detail: `![](${localImagePath})`,
  completed: false,
  resolution: null,
  comment: null,
}

const cardImageCases: {
  name: string
  message: ChatItem
  includeImageSources: boolean
  expectedMarkup: string
}[] = [
  {
    name: 'to-do detail',
    message: todoMessage,
    includeImageSources: true,
    expectedMarkup: expectedTodoCardMarkup(mappedImageMarkup),
  },
  {
    name: 'to-do without an image source',
    message: todoMessage,
    includeImageSources: false,
    expectedMarkup: expectedTodoCardMarkup(unavailableImageMarkup),
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
    includeImageSources: true,
    expectedMarkup: [
      '<article class="chat-proposal-card" aria-label="Proposal" aria-busy="false" data-answerable="true"><h2>Proposal · waiting for your choice</h2><h3>Choose a layout</h3>',
      '<div class="markdown-message chat-proposal-summary"><p><button class="image-lightbox-trigger" type="button" aria-label="Enlarge image"><img class="chat-user-image" src="/api/sessions/fixture-session/answer-images/fixture-image" alt="" loading="lazy" decoding="async"/></button></p></div><div class="chat-proposal-options" role="radiogroup" aria-label="Proposal options"></div>',
      '<label class="chat-proposal-comment-label" for="proposal-GENERATED_ID-comment">Comment (optional)</label><div class="chat-card-comment-drop-zone"><textarea id="proposal-GENERATED_ID-comment" class="chat-card-comment chat-proposal-comment" rows="2"></textarea></div><p class="conversation-error" role="status">Browser storage is unavailable. Drafts may not survive a page reload.</p>',
      '<footer class="chat-proposal-footer"><div class="chat-proposal-actions"><button type="button" data-disabled="" tabindex="0" disabled="" data-slot="button" class="group/button inline-flex shrink-0 items-center justify-center outline-none select-none focus-visible:ring-3 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50 aria-invalid:ring-3 aria-invalid:ring-destructive/40 [&amp;_svg]:pointer-events-none [&amp;_svg]:shrink-0 [&amp;_svg:not([class*=&#x27;size-&#x27;])]:size-4 text-muted-foreground hover:text-destructive aw-button aw-button--primary">Choose</button><button type="button" tabindex="0" data-slot="button" class="group/button inline-flex shrink-0 items-center justify-center outline-none select-none focus-visible:ring-3 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50 aria-invalid:ring-3 aria-invalid:ring-destructive/40 [&amp;_svg]:pointer-events-none [&amp;_svg]:shrink-0 [&amp;_svg:not([class*=&#x27;size-&#x27;])]:size-4 text-muted-foreground hover:text-destructive aw-button aw-button--outline">Ask for another option</button></div><span class="chat-proposal-hint">1 / 0 to pick · ⌘↵ to send</span></footer></article>',
    ].join(''),
  },
]

it.each(cardImageCases)(
  'renders the expected image state in a $name card',
  ({ message, includeImageSources, expectedMarkup }) => {
    expect(renderConversationCardMarkup(message, includeImageSources)).toBe(
      expectedMarkup,
    )
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
