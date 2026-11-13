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

const validFile = {
  name: 'sample-layout.png',
  source: '/api/sessions/sample-session/files/sample-file',
  isImage: true,
  available: true,
}

const fileAttachment = (overrides: Record<string, unknown> = {}) => ({
  type: 'file_attachment',
  files: [validFile],
  caption: null,
  ...overrides,
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

it.each([
  {
    name: 'a caption and timestamp',
    message: fileAttachment({
      caption: 'Review this image.',
      timestamp: '2030-09-12T03:00:00Z',
    }),
  },
  {
    name: 'a null caption without a timestamp',
    message: fileAttachment(),
  },
])('accepts a file attachment with $name', ({ message }) => {
  expect(isConversationResponse(response(message))).toBe(true)
})

it.each([
  {
    name: 'files that are not an array',
    message: fileAttachment({ files: 'sample-layout.png' }),
  },
  {
    name: 'a file without a name',
    message: fileAttachment({ files: [{ ...validFile, name: undefined }] }),
  },
  {
    name: 'a file with a non-string source',
    message: fileAttachment({ files: [{ ...validFile, source: 1 }] }),
  },
  {
    name: 'a file with a non-boolean image flag',
    message: fileAttachment({ files: [{ ...validFile, isImage: 'yes' }] }),
  },
  {
    name: 'a file with a non-boolean availability flag',
    message: fileAttachment({ files: [{ ...validFile, available: 1 }] }),
  },
  {
    name: 'a missing caption',
    message: fileAttachment({ caption: undefined }),
  },
  {
    name: 'a non-string, non-null caption',
    message: fileAttachment({ caption: 1 }),
  },
  {
    name: 'a non-string timestamp',
    message: fileAttachment({ timestamp: 1 }),
  },
])('rejects a file attachment with $name', ({ message }) => {
  expect(isConversationResponse(response(message))).toBe(false)
})
