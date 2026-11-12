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
