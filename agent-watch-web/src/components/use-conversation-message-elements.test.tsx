import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { useConversationMessageElements } from '#components/use-conversation-message-elements'
import type { ChatItem } from '#types'

const localImagePath = '/Users/guest/screenshot.png'
const servedImageSource =
  '/api/sessions/fixture-session/answer-images/fixture-image'

const messages: ChatItem[] = [
  { type: 'assistant', text: 'Earlier message.' },
  {
    type: 'todo',
    seq: 1,
    title: 'Review the draft',
    detail: `![](${localImagePath})`,
    completed: false,
    resolution: null,
    comment: null,
  },
]

function MessageImageSourceProbe() {
  const conversation = useConversationMessageElements({
    messages,
    messageImageSources: {
      '18': { [localImagePath]: servedImageSource },
    },
    start: 17,
    sessionCategory: 'in_progress',
    sessionId: 'fixture-session',
    isCompacting: false,
    sessionEngine: 'claude',
    actions: null,
    actionsReady: true,
    isMobile: false,
    selectedDecisionSeq: null,
    onSelectDecision: () => undefined,
    onAction: () => Promise.resolve(true),
    onOpenInTerminal: () => undefined,
    openInTerminalStatus: null,
    onSelectSession: () => undefined,
    pendingActions: {},
    sessions: [],
    now: new Date('2030-01-02T03:04:05.000Z'),
  })

  return (
    <pre>
      {JSON.stringify({
        indices: conversation.conversationMessageIndices,
        imageSources: conversation.elements.map(
          (element) => element.props.imageSources ?? null,
        ),
      })}
    </pre>
  )
}

it('routes a card image map from its absolute conversation index', () => {
  expect(renderToStaticMarkup(<MessageImageSourceProbe />)).toBe(
    '<pre>{&quot;indices&quot;:[17,18],&quot;imageSources&quot;:[null,{&quot;/Users/guest/screenshot.png&quot;:&quot;/api/sessions/fixture-session/answer-images/fixture-image&quot;}]}</pre>',
  )
})
