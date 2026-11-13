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
        resolution: 'rejected',
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
        resolution: null,
        comment: null,
      }),
    ),
  ).toBe(false)
})

it('accepts a permission denial card from the conversation API', () => {
  expect(
    isConversationResponse(
      response({
        type: 'permission_denied',
        seq: 1,
        toolUseId: 'tool-call-example',
        tool: 'Bash',
        inputSummary: 'printf example',
        reason: '[Sensitive Operation]',
        retryable: true,
        resolved: false,
      }),
    ),
  ).toBe(true)
})

it('rejects permission denial cards with an invalid retry state', () => {
  expect(
    isConversationResponse(
      response({
        type: 'permission_denied',
        seq: 1,
        toolUseId: 'tool-call-example',
        tool: 'Bash',
        inputSummary: 'printf example',
        reason: null,
        retryable: 'yes',
        resolved: false,
      }),
    ),
  ).toBe(false)
})
