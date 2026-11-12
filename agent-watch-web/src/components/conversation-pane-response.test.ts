import { expect, it } from 'vitest'

import { isConversationResponse } from '#components/conversation-pane-response'

const response = (message: unknown) => ({
  messages: [message],
  backgroundTasks: [],
  start: 0,
  total: 1,
  decisions: [],
  hasUnansweredProposal: false,
  activeQuestionIndex: null,
})

it('accepts a compact boundary event from the conversation API', () => {
  expect(
    isConversationResponse({
      messages: [
        {
          type: 'event',
          kind: 'compact_boundary',
          text: 'Context compacted',
          failed: false,
        },
      ],
      backgroundTasks: [],
      start: 0,
      total: 1,
      decisions: [],
      hasUnansweredProposal: false,
      activeQuestionIndex: null,
    }),
  ).toBe(true)
})

it('accepts todo cards with completion metadata', () => {
  expect(
    isConversationResponse(
      response({
        type: 'todo',
        seq: 1,
        title: 'Check the page',
        detail: 'Open it on a phone.',
        completed: true,
        comment: 'Done',
      }),
    ),
  ).toBe(true)
})

it('rejects todo cards with invalid completion state', () => {
  expect(
    isConversationResponse(
      response({
        type: 'todo',
        seq: 1,
        title: 'Check the page',
        detail: null,
        completed: 'yes',
        comment: null,
      }),
    ),
  ).toBe(false)
})
