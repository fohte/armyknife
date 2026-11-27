import { expect, it } from 'vitest'

import {
  isConversationResponse,
  isReviewDocumentResponse,
  isSessionTreeResponse,
} from '#components/conversation-pane-response'

const response = (message: unknown) => ({
  messages: [message],
  backgroundTasks: [],
  start: 0,
  total: 1,
  hasUnansweredProposal: false,
  activeQuestionIndex: null,
})

const validFile = {
  name: 'sample-layout.png',
  source: '/api/sessions/sample-session/files/sample-file',
  size: 1024,
  isImage: true,
  previewType: 'image',
  available: true,
}

const fileAttachment = (overrides: Record<string, unknown> = {}) => ({
  type: 'file_attachment',
  files: [validFile],
  caption: null,
  ...overrides,
})

const groupedToolSummary = (items: unknown[]) => ({
  type: 'tool_summary',
  summary: 'Read ×1',
  operations: [{ tool: 'Read', target: 'fixture/source.rs', failed: false }],
  items,
})

it.each([
  {
    name: 'accepts related issue titles',
    response: {
      id: 'fixture-review',
      revision: 'fixture-revision',
      frontmatter: { parentIssue: 'sample-owner/sample-repo#42' },
      body: 'Sample issue body.',
      relatedIssueTitles: {
        'sample-owner/sample-repo#42': 'Sample parent title',
      },
    },
    expected: true,
  },
  {
    name: 'accepts an older response without related issue titles',
    response: {
      id: 'fixture-review',
      revision: 'fixture-revision',
      frontmatter: { parentIssue: 'sample-owner/sample-repo#42' },
      body: 'Sample issue body.',
    },
    expected: true,
  },
  {
    name: 'rejects a non-string related issue title',
    response: {
      id: 'fixture-review',
      revision: 'fixture-revision',
      frontmatter: { parentIssue: 'sample-owner/sample-repo#42' },
      body: 'Sample issue body.',
      relatedIssueTitles: { 'sample-owner/sample-repo#42': 42 },
    },
    expected: false,
  },
])('validates review-document $name', ({ response, expected }) => {
  expect(isReviewDocumentResponse(response)).toEqual(expected)
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
      hasUnansweredProposal: false,
      activeQuestionIndex: null,
    }),
  ).toBe(true)
})

it.each([
  {
    name: 'string IDs',
    confirmedQueuedPromptIds: ['fixture-message-id'],
    expected: true,
  },
  {
    name: 'non-string IDs',
    confirmedQueuedPromptIds: [42],
    expected: false,
  },
])(
  'validates $name for confirmed queued prompts',
  ({ confirmedQueuedPromptIds, expected }) => {
    expect(
      isConversationResponse({
        ...response({
          type: 'event',
          kind: 'system',
          text: 'A system event',
          failed: false,
        }),
        confirmedQueuedPromptIds,
      }),
    ).toEqual(expected)
  },
)

it.each([
  {
    name: 'inherited vibe mode',
    vibeMode: { mode: 'inherited', ancestorSessionId: 'fixture-parent' },
    expected: true,
  },
  {
    name: 'vibe mode with a missing ancestor id',
    vibeMode: { mode: 'inherited' },
    expected: false,
  },
])('validates $name from the conversation API', ({ vibeMode, expected }) => {
  expect(
    isConversationResponse({
      messages: [],
      backgroundTasks: [],
      start: 0,
      total: 0,
      hasUnansweredProposal: false,
      activeQuestionIndex: null,
      vibeMode,
    }),
  ).toEqual(expected)
})

it.each([
  {
    name: 'a tree node with vibe mode',
    vibeMode: { mode: 'enabled' },
    expected: true,
  },
  {
    name: 'a tree node without vibe mode',
    vibeMode: undefined,
    expected: false,
  },
])('validates $name', ({ vibeMode, expected }) => {
  expect(
    isSessionTreeResponse({
      revision: 7,
      sessions: [
        {
          id: 'fixture-session',
          parentId: null,
          category: 'in_progress',
          isActive: true,
          kind: null,
          title: 'Fixture session',
          preview: null,
          elapsedSeconds: 0,
          task: null,
          repo: 'sample/repo',
          ...(vibeMode === undefined ? {} : { vibeMode }),
          pullRequest: null,
        },
      ],
    }),
  ).toEqual(expected)
})

it('accepts narration and grouped thinking from the conversation API', () => {
  expect(
    isConversationResponse({
      messages: [
        {
          type: 'narration',
          text: 'I will inspect the fictional source.',
          thinkingDurationMs: 47,
        },
        {
          type: 'tool_summary',
          summary: 'Read ×1',
          operations: [
            { tool: 'Read', target: 'fixture/source.rs', failed: false },
          ],
          items: [
            {
              type: 'thinking',
              text: 'Review the fictional request.',
              thinkingDurationMs: 63,
            },
            {
              type: 'tool_use',
              tool: 'Read',
              target: 'fixture/source.rs',
              failed: false,
            },
          ],
        },
        {
          type: 'tool_in_progress',
          tool: 'Read',
          target: 'fixture/source.rs',
          items: [
            {
              type: 'thinking',
              text: 'Review the fictional request.',
              thinkingDurationMs: 63,
            },
            {
              type: 'tool_use',
              tool: 'Read',
              target: 'fixture/source.rs',
              failed: false,
            },
          ],
        },
        {
          type: 'permission',
          tool: 'Read',
          description: '',
          items: [
            {
              type: 'thinking',
              text: 'Review the fictional request.',
            },
            {
              type: 'tool_use',
              tool: 'Read',
              failed: false,
            },
          ],
        },
      ],
      backgroundTasks: [],
      start: 0,
      total: 4,
      hasUnansweredProposal: false,
      activeQuestionIndex: null,
    }),
  ).toBe(true)
})

it.each([
  {
    name: 'a tool use with a non-boolean failure state',
    message: response(
      groupedToolSummary([
        {
          type: 'tool_use',
          tool: 'Read',
          target: 'fixture/source.rs',
          failed: 'false',
        },
      ]),
    ),
  },
  {
    name: 'thinking with a negative duration',
    message: response(
      groupedToolSummary([
        {
          type: 'thinking',
          text: 'Review the fictional request.',
          thinkingDurationMs: -1,
        },
      ]),
    ),
  },
])('rejects grouped items with $name', ({ message }) => {
  expect(isConversationResponse(message)).toBe(false)
})

it('accepts an epoch response without an older cursor', () => {
  expect(
    isConversationResponse({
      ...response({ type: 'assistant', text: 'Fixture response.' }),
      epoch: 'fixture-epoch-amber',
      epochReset: true,
    }),
  ).toBe(true)
})

it.each([
  { name: 'a numeric epoch', field: 'epoch', value: 1 },
  { name: 'a numeric older cursor', field: 'olderCursor', value: 1 },
  { name: 'a null older cursor', field: 'olderCursor', value: null },
  { name: 'a string epoch reset flag', field: 'epochReset', value: 'true' },
])('rejects $name', ({ field, value }) => {
  expect(
    isConversationResponse({
      ...response({ type: 'assistant', text: 'Fixture response.' }),
      [field]: value,
    }),
  ).toBe(false)
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

it('accepts a queued prompt ID on a delivered user message', () => {
  expect(
    isConversationResponse(
      response({
        type: 'user',
        text: 'A message',
        queuedPromptId: 'fixture-message-id',
      }),
    ),
  ).toBe(true)
})

it('rejects a non-string queued prompt ID on a delivered user message', () => {
  expect(
    isConversationResponse(
      response({
        type: 'user',
        text: 'A message',
        queuedPromptId: 1,
      }),
    ),
  ).toBe(false)
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
        resolution: null,
      }),
    ),
  ).toBe(true)
})

it('accepts a dismissed permission denial from the conversation API', () => {
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
        resolved: true,
        resolution: 'dismissed',
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
        resolution: null,
      }),
    ),
  ).toBe(false)
})

it('rejects permission denial cards with an unknown resolution', () => {
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
        resolved: true,
        resolution: 'withdrawn',
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
    name: 'a file without a size',
    message: fileAttachment({ files: [{ ...validFile, size: undefined }] }),
  },
  {
    name: 'a file with a non-boolean image flag',
    message: fileAttachment({ files: [{ ...validFile, isImage: 'yes' }] }),
  },
  {
    name: 'a file with a negative size',
    message: fileAttachment({ files: [{ ...validFile, size: -1 }] }),
  },
  {
    name: 'a file with a non-boolean availability flag',
    message: fileAttachment({ files: [{ ...validFile, available: 1 }] }),
  },
  {
    name: 'a file with an unknown preview type',
    message: fileAttachment({
      files: [{ ...validFile, previewType: 'script' }],
    }),
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
