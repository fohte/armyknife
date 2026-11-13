import { afterEach, expect, it, vi } from 'vitest'

import {
  applyConversationOperation,
  applySessionOperation,
  createSessionOperation,
  isConversationOperationConfirmed,
  type SessionOperation,
  sessionOperationKey,
  submitSessionOperation,
} from '#session-operations'
import type { ChatItem, ConversationResponse, Session } from '#types'

const session: Session = {
  id: 'test-session',
  category: 'needs_action',
  isPaused: false,
  kind: null,
  title: 'Sample task',
  preview: null,
  elapsedSeconds: 0,
  unread: false,
  task: { id: 'task-old', number: 1, title: 'Old task', priority: null },
  tasks: [
    { id: 'task-old', number: 1, title: 'Old task', priority: null },
    { id: 'task-new', number: 2, title: 'New task', priority: null },
  ],
  repo: 'sample/repository',
  project: 'Sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const questionMessage: Extract<ChatItem, { type: 'question' }> = {
  type: 'question',
  requestId: 'question-request',
  questions: [
    {
      question: 'Choose an option',
      choices: [],
      multi_select: false,
      answer: null,
    },
  ],
}
const decisionMessage: Extract<ChatItem, { type: 'decision' }> = {
  type: 'decision',
  seq: 2,
  title: 'Decision',
  why: 'Reason',
  considered: [],
  easyToChange: 'Yes',
  review: null,
  createdAt: '2026-01-01T00:00:00Z',
}
const proposalMessage: Extract<ChatItem, { type: 'proposal' }> = {
  type: 'proposal',
  seq: 3,
  title: 'Proposal',
  summary: null,
  options: [],
  answer: null,
  resolved: false,
}
const todoMessage: Extract<ChatItem, { type: 'todo' }> = {
  type: 'todo',
  seq: 4,
  title: 'Todo',
  detail: null,
  completed: false,
  comment: null,
}
const deniedMessage: Extract<ChatItem, { type: 'permission_denied' }> = {
  type: 'permission_denied',
  seq: 5,
  tool: 'Shell',
  inputSummary: 'run command',
  reason: null,
  retryable: true,
  resolved: false,
}
const conversation: ConversationResponse = {
  messages: [
    questionMessage,
    decisionMessage,
    proposalMessage,
    todoMessage,
    deniedMessage,
  ],
  backgroundTasks: [],
  start: 0,
  total: 5,
  decisions: [decisionMessage],
  hasUnansweredProposal: true,
  activeQuestionIndex: 0,
}

const requests: {
  name: string
  operation: SessionOperation
  path: string
  body?: object
  response?: unknown
}[] = [
  {
    name: 'star session',
    operation: { type: 'today', sessionId: 'test-session', isToday: true },
    path: 'today',
    body: { isToday: true },
  },
  {
    name: 'move session later',
    operation: {
      type: 'later',
      sessionId: 'test-session',
      preset: 'tomorrow',
      expectedDeadline: '2030-01-02T09:00:00Z',
    },
    path: 'later',
    body: {
      preset: 'tomorrow',
      expectedDeadline: '2030-01-02T09:00:00Z',
    },
  },
  {
    name: 'return session now',
    operation: { type: 'back_now', sessionId: 'test-session' },
    path: 'later',
    body: { preset: 'back_now' },
  },
  {
    name: 'close session',
    operation: {
      type: 'close',
      sessionId: 'test-session',
      keepWorktree: false,
    },
    path: 'close',
    body: { keepWorktree: false },
  },
  {
    name: 'focus terminal',
    operation: { type: 'focus', sessionId: 'test-session' },
    path: 'focus',
  },
  {
    name: 'interrupt session',
    operation: { type: 'interrupt', sessionId: 'test-session' },
    path: 'interrupt',
  },
  {
    name: 'send message',
    operation: {
      type: 'message',
      sessionId: 'test-session',
      content: 'A message',
    },
    path: 'messages',
    body: { content: 'A message' },
  },
  {
    name: 'answer question',
    operation: {
      type: 'answer',
      sessionId: 'test-session',
      requestId: 'question-request',
      answers: { 'Choose an option': 'Option A' },
    },
    path: 'answer',
    body: {
      requestId: 'question-request',
      answers: { 'Choose an option': 'Option A' },
    },
  },
  {
    name: 'skip question',
    operation: {
      type: 'answer',
      sessionId: 'test-session',
      requestId: 'question-request',
      skip: true,
    },
    path: 'answer',
    body: { requestId: 'question-request', skip: true },
  },
  {
    name: 'approve decision',
    operation: { type: 'decision_ok', sessionId: 'test-session', seq: 2 },
    path: 'decisions/2/ok',
  },
  {
    name: 'request decision change',
    operation: {
      type: 'decision_change',
      sessionId: 'test-session',
      seq: 2,
      comment: 'Change this decision',
    },
    path: 'decisions/2/change',
    body: { comment: 'Change this decision' },
  },
  {
    name: 'answer proposal',
    operation: {
      type: 'proposal_answer',
      sessionId: 'test-session',
      seq: 3,
      choice: 0,
      comment: null,
    },
    path: 'proposals/3/answer',
    body: { choice: 0, comment: null },
  },
  {
    name: 'complete todo',
    operation: {
      type: 'todo_complete',
      sessionId: 'test-session',
      seq: 4,
      comment: 'Done',
    },
    path: 'todos/4/complete',
    body: { comment: 'Done' },
  },
  {
    name: 'approve permission',
    operation: {
      type: 'permission',
      sessionId: 'test-session',
      action: 'approve',
      requestId: 'permission-request',
    },
    path: 'permission/approve',
    body: { requestId: 'permission-request' },
  },
  {
    name: 'reject permission',
    operation: {
      type: 'permission',
      sessionId: 'test-session',
      action: 'reject',
      requestId: 'permission-request',
    },
    path: 'permission/reject',
    body: { requestId: 'permission-request' },
  },
  {
    name: 'retry denied permission',
    operation: { type: 'permission_retry', sessionId: 'test-session', seq: 5 },
    path: 'permission-denials/5/retry',
  },
  {
    name: 'change primary task',
    operation: {
      type: 'primary_task',
      sessionId: 'test-session',
      taskId: 'task-new',
    },
    path: 'primary-task',
    body: { taskId: 'task-new' },
  },
  {
    name: 'merge pull request',
    operation: {
      type: 'merge',
      sessionId: 'test-session',
      number: 9,
      owner: 'sample-owner',
      repo: 'sample-repository',
      expectedHeadSha: 'sample-sha',
    },
    path: 'pull-requests/9/merge',
    body: {
      owner: 'sample-owner',
      repo: 'sample-repository',
      expectedHeadSha: 'sample-sha',
    },
    response: { result: 'merged' },
  },
]

afterEach(() => {
  vi.unstubAllGlobals()
})

const operationFactories: [
  string,
  string,
  Record<string, unknown>,
  SessionOperation,
][] = [
  [
    'Starred',
    'today',
    { isToday: true },
    { type: 'today', sessionId: session.id, isToday: true },
  ],
  [
    'Later',
    'later',
    { preset: 'tomorrow', expectedDeadline: '2030-01-02T09:00:00Z' },
    {
      type: 'later',
      sessionId: session.id,
      preset: 'tomorrow',
      expectedDeadline: '2030-01-02T09:00:00Z',
    },
  ],
  [
    'Back now',
    'later',
    { preset: 'back_now' },
    { type: 'back_now', sessionId: session.id },
  ],
  [
    'close',
    'close',
    { keepWorktree: false },
    { type: 'close', sessionId: session.id, keepWorktree: false },
  ],
  ['terminal focus', 'focus', {}, { type: 'focus', sessionId: session.id }],
  ['interrupt', 'interrupt', {}, { type: 'interrupt', sessionId: session.id }],
  [
    'message',
    'messages',
    { content: 'A message', _messageCountBefore: 2 },
    {
      type: 'message',
      sessionId: session.id,
      content: 'A message',
      messageCountBefore: 2,
    },
  ],
  [
    'question answer',
    'answer',
    {
      requestId: 'question-request',
      answers: { 'Choose an option': 'Option A' },
    },
    {
      type: 'answer',
      sessionId: session.id,
      requestId: 'question-request',
      answers: { 'Choose an option': 'Option A' },
    },
  ],
  [
    'question skip',
    'answer',
    { requestId: 'question-request', skip: true },
    {
      type: 'answer',
      sessionId: session.id,
      requestId: 'question-request',
      skip: true,
    },
  ],
  [
    'decision approval',
    'decisions/2/ok',
    {},
    { type: 'decision_ok', sessionId: session.id, seq: 2 },
  ],
  [
    'decision change',
    'decisions/2/change',
    { comment: 'Change this decision' },
    {
      type: 'decision_change',
      sessionId: session.id,
      seq: 2,
      comment: 'Change this decision',
    },
  ],
  [
    'proposal answer',
    'proposals/3/answer',
    { choice: 0, comment: null },
    {
      type: 'proposal_answer',
      sessionId: session.id,
      seq: 3,
      choice: 0,
      comment: null,
    },
  ],
  [
    'todo completion',
    'todos/4/complete',
    { comment: 'Done' },
    { type: 'todo_complete', sessionId: session.id, seq: 4, comment: 'Done' },
  ],
  [
    'permission approval',
    'permission/approve',
    { requestId: 'permission-request' },
    {
      type: 'permission',
      sessionId: session.id,
      action: 'approve',
      requestId: 'permission-request',
    },
  ],
  [
    'permission rejection',
    'permission/reject',
    { requestId: 'permission-request' },
    {
      type: 'permission',
      sessionId: session.id,
      action: 'reject',
      requestId: 'permission-request',
    },
  ],
  [
    'denied action retry',
    'permission-denials/5/retry',
    {},
    { type: 'permission_retry', sessionId: session.id, seq: 5 },
  ],
  [
    'primary task change',
    'primary-task',
    { taskId: 'task-new' },
    { type: 'primary_task', sessionId: session.id, taskId: 'task-new' },
  ],
]

it.each(operationFactories)(
  '%s uses its operation definition',
  (_name, action, payload, expected) => {
    const actual = createSessionOperation(session.id, action, payload).match(
      (operation) => operation,
      (error) => ({ error: error.message }),
    )
    expect(actual).toEqual(expected)
  },
)

it('keeps confirmation waiters distinct for different target values', () => {
  const taskA: SessionOperation = {
    type: 'primary_task',
    sessionId: session.id,
    taskId: 'task-a',
  }
  const taskB: SessionOperation = { ...taskA, taskId: 'task-b' }
  const keys = () => [sessionOperationKey(taskA), sessionOperationKey(taskB)]

  expect(keys()).toEqual([
    'test-session:primary_task:task-a',
    'test-session:primary_task:task-b',
  ])
})

it.each(requests)(
  '$name uses its session operation definition',
  async (testCase) => {
    const response =
      testCase.response === undefined
        ? new Response(null, { status: 204 })
        : Response.json(testCase.response)
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(response)
    vi.stubGlobal('fetch', fetchMock)

    const result = await submitSessionOperation(testCase.operation)
    const requestUrl = `/api/sessions/test-session/${testCase.path}`
    const requestInit =
      testCase.body === undefined
        ? { method: 'POST' }
        : {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(testCase.body),
          }

    if (result.isErr()) throw result.error
    expect(fetchMock.mock.calls).toEqual([[requestUrl, requestInit]])
  },
)

const sessionReducerCases: [
  string,
  SessionOperation,
  readonly Session[],
  Session[],
][] = [
  [
    'sets Starred',
    { type: 'today', sessionId: session.id, isToday: true },
    [session],
    [{ ...session, isToday: true }],
  ],
  [
    'moves to Later',
    {
      type: 'later',
      sessionId: session.id,
      preset: 'tomorrow',
      expectedDeadline: '2030-01-02T09:00:00Z',
    },
    [session],
    [
      {
        ...session,
        category: 'later',
        categoryWhenNotLater: 'needs_action',
        laterUntil: '2030-01-02T09:00:00Z',
      },
    ],
  ],
  [
    'returns from Later',
    { type: 'back_now', sessionId: session.id },
    [
      {
        ...session,
        category: 'later',
        categoryWhenNotLater: 'in_progress',
        laterUntil: '2030-01-02T09:00:00Z',
      },
    ],
    [{ ...session, category: 'in_progress', laterUntil: null }],
  ],
  [
    'keeps an already returned session unchanged',
    { type: 'back_now', sessionId: session.id },
    [{ ...session, category: 'in_progress', laterUntil: null }],
    [{ ...session, category: 'in_progress', laterUntil: null }],
  ],
  [
    'closes the session',
    { type: 'close', sessionId: session.id, keepWorktree: false },
    [session],
    [],
  ],
  [
    'changes primary task',
    { type: 'primary_task', sessionId: session.id, taskId: 'task-new' },
    [session],
    [{ ...session, task: session.tasks?.[1] ?? null }],
  ],
]

it.each(sessionReducerCases)(
  '%s immediately changes the session view',
  (_name, operation, base, expected) => {
    expect(applySessionOperation(base, operation)).toEqual(expected)
  },
)

const decisionList = [decisionMessage]
const conversationReducerCases: [
  string,
  SessionOperation,
  ConversationResponse,
][] = [
  [
    'sends a message',
    {
      type: 'message',
      sessionId: session.id,
      content: 'Inspect this image.',
      attachments: [
        {
          name: 'sample-image.png',
          mimeType: 'image/png',
          data: 'AQID',
          size: 3,
        },
      ],
      messageCountBefore: 5,
    },
    {
      ...conversation,
      messages: [
        ...conversation.messages,
        {
          type: 'queued_user',
          content: [
            { type: 'text', text: 'Inspect this image.' },
            {
              type: 'image',
              source: 'data:image/png;base64,AQID',
              available: true,
            },
          ],
        },
      ],
      total: 6,
    },
  ],
  [
    'answers a question',
    {
      type: 'answer',
      sessionId: session.id,
      requestId: 'question-request',
      answers: { 'Choose an option': 'Option A' },
    },
    {
      ...conversation,
      messages: [
        {
          ...questionMessage,
          questions: questionMessage.questions.map((question) => ({
            ...question,
            answer: 'Option A',
          })),
        },
        decisionMessage,
        proposalMessage,
        todoMessage,
        deniedMessage,
      ],
      decisions: decisionList,
    },
  ],
  [
    'skips a question',
    {
      type: 'answer',
      sessionId: session.id,
      requestId: 'question-request',
      skip: true,
    },
    {
      ...conversation,
      messages: [decisionMessage, proposalMessage, todoMessage, deniedMessage],
      decisions: decisionList,
      activeQuestionIndex: null,
    },
  ],
  [
    'approves a decision',
    { type: 'decision_ok', sessionId: session.id, seq: 2 },
    {
      ...conversation,
      messages: [
        questionMessage,
        { ...decisionMessage, review: 'ok' },
        proposalMessage,
        todoMessage,
        deniedMessage,
      ],
      decisions: [{ ...decisionMessage, review: 'ok' }],
    },
  ],
  [
    'requests a decision change',
    {
      type: 'decision_change',
      sessionId: session.id,
      seq: 2,
      comment: 'Change this decision',
    },
    {
      ...conversation,
      messages: [
        questionMessage,
        { ...decisionMessage, review: 'change' },
        proposalMessage,
        todoMessage,
        deniedMessage,
      ],
      decisions: [{ ...decisionMessage, review: 'change' }],
    },
  ],
  [
    'answers a proposal',
    {
      type: 'proposal_answer',
      sessionId: session.id,
      seq: 3,
      choice: 0,
      comment: null,
    },
    {
      ...conversation,
      messages: [
        questionMessage,
        decisionMessage,
        {
          ...proposalMessage,
          answer: { choice: 0, comment: null },
          resolved: true,
        },
        todoMessage,
        deniedMessage,
      ],
      decisions: decisionList,
      hasUnansweredProposal: false,
    },
  ],
  [
    'completes a todo',
    {
      type: 'todo_complete',
      sessionId: session.id,
      seq: 4,
      comment: 'Done',
    },
    {
      ...conversation,
      messages: [
        questionMessage,
        decisionMessage,
        proposalMessage,
        { ...todoMessage, completed: true, comment: 'Done' },
        deniedMessage,
      ],
      decisions: decisionList,
    },
  ],
  [
    'resolves a denied action retry',
    { type: 'permission_retry', sessionId: session.id, seq: 5 },
    {
      ...conversation,
      messages: [
        questionMessage,
        decisionMessage,
        proposalMessage,
        todoMessage,
        { ...deniedMessage, resolved: true },
      ],
      decisions: decisionList,
    },
  ],
]

it.each(conversationReducerCases)(
  '%s immediately changes the conversation view',
  (_name, operation, expected) => {
    expect(applyConversationOperation(conversation, operation)).toEqual(
      expected,
    )
  },
)

it('replaces an optimistic message when the server returns it', () => {
  const operation: SessionOperation = {
    type: 'message',
    sessionId: session.id,
    content: 'A message',
    messageCountBefore: conversation.total,
  }
  const serverConversation: ConversationResponse = {
    ...conversation,
    messages: [...conversation.messages, { type: 'user', text: 'A message' }],
    total: conversation.total + 1,
  }

  expect(applyConversationOperation(serverConversation, operation)).toEqual(
    serverConversation,
  )
})

it('confirms a message after it appears in the server response', () => {
  const operation: SessionOperation = {
    type: 'message',
    sessionId: session.id,
    content: 'A message',
    messageCountBefore: conversation.total,
  }
  const serverConversation: ConversationResponse = {
    ...conversation,
    messages: [...conversation.messages, { type: 'user', text: 'A message' }],
    total: conversation.total + 1,
  }

  expect(
    isConversationOperationConfirmed(serverConversation, operation, null),
  ).toBe(true)
})
