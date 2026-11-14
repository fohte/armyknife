import { afterEach, expect, it, vi } from 'vitest'

import {
  applyConversationOperation,
  applySessionOperation,
  createSessionOperation,
  isConversationOperationConfirmed,
  isSessionOperationConfirmed,
  requiresStateConfirmation,
  type SessionOperation,
  sessionOperationKey,
  submitSessionOperation,
} from '#session-operations'
import type {
  ChatItem,
  ConversationResponse,
  Session,
  SessionActions,
} from '#types'

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
  resolution: null,
  comment: null,
}
const todoMessage: Extract<ChatItem, { type: 'todo' }> = {
  type: 'todo',
  seq: 4,
  title: 'Todo',
  detail: null,
  completed: false,
  resolution: null,
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
    name: 'set session title',
    operation: {
      type: 'title',
      sessionId: 'test-session',
      title: 'A clearer title',
    },
    path: 'title',
    body: { title: 'A clearer title' },
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
    name: 'preview agent commands',
    operation: {
      type: 'command_preview',
      sessionId: 'test-session',
      messageText: 'Run `! printf safe`.',
      commands: ['printf safe'],
    },
    path: 'commands/preview',
    body: {
      messageText: 'Run `! printf safe`.',
      commands: ['printf safe'],
    },
    response: { cwd: '/workspace/example' },
  },
  {
    name: 'execute agent commands',
    operation: {
      type: 'command_execute',
      sessionId: 'test-session',
      messageText: 'Run `! printf safe`.',
      commands: ['printf safe'],
    },
    path: 'commands/execute',
    body: {
      messageText: 'Run `! printf safe`.',
      commands: ['printf safe'],
    },
    response: { exitCode: 0, succeeded: true, timedOut: false },
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
      type: 'todo_resolve',
      sessionId: 'test-session',
      seq: 4,
      resolution: 'completed',
      comment: 'Done',
    },
    path: 'todos/4/resolve',
    body: { resolution: 'completed', comment: 'Done' },
  },
  {
    name: 'reject todo',
    operation: {
      type: 'todo_resolve',
      sessionId: 'test-session',
      seq: 4,
      resolution: 'rejected',
      comment: 'Not needed',
    },
    path: 'todos/4/resolve',
    body: { resolution: 'rejected', comment: 'Not needed' },
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
    name: 'save review document',
    operation: {
      type: 'review_document_save',
      sessionId: 'test-session',
      id: 'fixture-review',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
    },
    path: 'review-document/save',
    body: {
      id: 'fixture-review',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
    },
  },
  {
    name: 'finish review document',
    operation: {
      type: 'review_document_finish',
      sessionId: 'test-session',
      id: 'fixture-review',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
    },
    path: 'review-document/finish',
    body: {
      id: 'fixture-review',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
    },
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
  {
    name: 'rewind conversation',
    operation: {
      type: 'rewind',
      sessionId: 'test-session',
      messageId: 'fixture-user-message',
      mode: 'conversation',
    },
    path: 'rewind',
    body: { messageId: 'fixture-user-message', mode: 'conversation' },
    response: { sessionId: 'fixture-rewound-session' },
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
    'title change',
    'title',
    { title: 'A clearer title' },
    { type: 'title', sessionId: session.id, title: 'A clearer title' },
  ],
  [
    'message',
    'messages',
    {
      content: 'A message',
      _messageCountBefore: 2,
      _operationId: 'message-fixture',
    },
    {
      type: 'message',
      sessionId: session.id,
      operationId: 'message-fixture',
      content: 'A message',
      messageCountBefore: 2,
    },
  ],
  [
    'command preview',
    'commands/preview',
    { messageText: 'Run `! printf safe`.', commands: ['printf safe'] },
    {
      type: 'command_preview',
      sessionId: session.id,
      messageText: 'Run `! printf safe`.',
      commands: ['printf safe'],
    },
  ],
  [
    'command execution',
    'commands/execute',
    { messageText: 'Run `! printf safe`.', commands: ['printf safe'] },
    {
      type: 'command_execute',
      sessionId: session.id,
      messageText: 'Run `! printf safe`.',
      commands: ['printf safe'],
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
    'todos/4/resolve',
    { resolution: 'completed', comment: 'Done' },
    {
      type: 'todo_resolve',
      sessionId: session.id,
      seq: 4,
      resolution: 'completed',
      comment: 'Done',
    },
  ],
  [
    'todo rejection',
    'todos/4/resolve',
    { resolution: 'rejected', comment: null },
    {
      type: 'todo_resolve',
      sessionId: session.id,
      seq: 4,
      resolution: 'rejected',
      comment: null,
    },
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
  [
    'save review document',
    'review-document/save',
    {
      id: 'fixture-review',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
    },
    {
      type: 'review_document_save',
      sessionId: session.id,
      id: 'fixture-review',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
    },
  ],
  [
    'finish review document',
    'review-document/finish',
    {
      id: 'fixture-review',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
    },
    {
      type: 'review_document_finish',
      sessionId: session.id,
      id: 'fixture-review',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
    },
  ],
  [
    'rewind conversation',
    'rewind',
    { messageId: 'fixture-user-message', mode: 'conversation' },
    {
      type: 'rewind',
      sessionId: session.id,
      messageId: 'fixture-user-message',
      mode: 'conversation',
    },
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
  const titleA: SessionOperation = {
    type: 'title',
    sessionId: session.id,
    title: 'Title A',
  }
  const titleB: SessionOperation = { ...titleA, title: 'Title B' }
  const completedTodo: SessionOperation = {
    type: 'todo_resolve',
    sessionId: session.id,
    seq: 4,
    resolution: 'completed',
    comment: null,
  }
  const rejectedTodo: SessionOperation = {
    ...completedTodo,
    resolution: 'rejected',
  }
  const previewA: SessionOperation = {
    type: 'command_preview',
    sessionId: session.id,
    messageText: 'Run `! printf a`.',
    commands: ['printf a'],
  }
  const previewB: SessionOperation = {
    ...previewA,
    commands: ['printf b'],
  }
  const keys = () => [
    sessionOperationKey(taskA),
    sessionOperationKey(taskB),
    sessionOperationKey(titleA),
    sessionOperationKey(titleB),
    sessionOperationKey(completedTodo),
    sessionOperationKey(rejectedTodo),
    sessionOperationKey(previewA),
    sessionOperationKey(previewB),
  ]

  expect(keys()).toEqual([
    'test-session:primary_task:task-a',
    'test-session:primary_task:task-b',
    'test-session:title:Title A',
    'test-session:title:Title B',
    'test-session:todo_resolve:4:completed:',
    'test-session:todo_resolve:4:rejected:',
    'test-session:command_preview:["Run `! printf a`.",["printf a"]]',
    'test-session:command_preview:["Run `! printf a`.",["printf b"]]',
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

    const actual = result.match(
      (value) => ({ type: 'success' as const, value }),
      (error) => ({ type: 'error' as const, message: error.message }),
    )
    const getSnapshot = () => ({ calls: fetchMock.mock.calls, result: actual })
    expect(getSnapshot()).toEqual({
      calls: [[requestUrl, requestInit]],
      result: { type: 'success', value: testCase.response },
    })
  },
)

it.each(requests.filter(({ response }) => response !== undefined))(
  '$name returns the response JSON',
  async (testCase) => {
    const response = Response.json(testCase.response)
    vi.stubGlobal('fetch', vi.fn<typeof fetch>().mockResolvedValue(response))
    const result = await submitSessionOperation(testCase.operation)
    const actual = result.match(
      (value) => value,
      (error) => error.message,
    )

    expect(actual).toEqual(testCase.response)
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
  [
    'changes session title',
    { type: 'title', sessionId: session.id, title: 'A clearer title' },
    [session],
    [{ ...session, title: 'A clearer title' }],
  ],
  [
    'leaves the title unchanged while clearing its manual override',
    { type: 'title', sessionId: session.id, title: '  ' },
    [session],
    [session],
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
      type: 'todo_resolve',
      sessionId: session.id,
      seq: 4,
      resolution: 'completed',
      comment: 'Done',
    },
    {
      ...conversation,
      messages: [
        questionMessage,
        decisionMessage,
        proposalMessage,
        {
          ...todoMessage,
          completed: true,
          resolution: 'completed',
          comment: 'Done',
        },
        deniedMessage,
      ],
      decisions: decisionList,
    },
  ],
  [
    'rejects a todo',
    {
      type: 'todo_resolve',
      sessionId: session.id,
      seq: 4,
      resolution: 'rejected',
      comment: 'Not needed',
    },
    {
      ...conversation,
      messages: [
        questionMessage,
        decisionMessage,
        proposalMessage,
        {
          ...todoMessage,
          completed: true,
          resolution: 'rejected',
          comment: 'Not needed',
        },
        deniedMessage,
      ],
      decisions: decisionList,
      hasUnansweredProposal: true,
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

const sessionListConfirmationCases: [
  string,
  SessionOperation,
  readonly Session[],
  boolean,
][] = [
  [
    'keeps Starred pending for an old session snapshot',
    { type: 'today', sessionId: session.id, isToday: true },
    [session],
    false,
  ],
  [
    'confirms Starred from a matching session snapshot',
    { type: 'today', sessionId: session.id, isToday: true },
    [{ ...session, isToday: true }],
    true,
  ],
  [
    'confirms Later from its scheduled deadline',
    {
      type: 'later',
      sessionId: session.id,
      preset: 'tomorrow',
      expectedDeadline: '2030-01-02T09:00:00Z',
    },
    [
      {
        ...session,
        category: 'later',
        categoryWhenNotLater: 'needs_action',
        laterUntil: '2030-01-02T09:00:00Z',
      },
    ],
    true,
  ],
  [
    'keeps Later pending when the session is still in its old category',
    {
      type: 'later',
      sessionId: session.id,
      preset: 'tomorrow',
      expectedDeadline: '2030-01-02T09:00:00Z',
    },
    [session],
    false,
  ],
  [
    'confirms Back now after the session leaves Later',
    { type: 'back_now', sessionId: session.id },
    [session],
    true,
  ],
  [
    'keeps Back now pending while the session remains in Later',
    { type: 'back_now', sessionId: session.id },
    [
      {
        ...session,
        category: 'later',
        categoryWhenNotLater: 'needs_action',
        laterUntil: '2030-01-02T09:00:00Z',
      },
    ],
    false,
  ],
  [
    'confirms Close when the session leaves the snapshot',
    { type: 'close', sessionId: session.id, keepWorktree: false },
    [],
    true,
  ],
  [
    'keeps Close pending while the session remains in the snapshot',
    { type: 'close', sessionId: session.id, keepWorktree: false },
    [session],
    false,
  ],
  [
    'confirms the selected primary task',
    { type: 'primary_task', sessionId: session.id, taskId: 'task-new' },
    [{ ...session, task: session.tasks?.[1] ?? null }],
    true,
  ],
  [
    'keeps a primary task change pending on the old task',
    { type: 'primary_task', sessionId: session.id, taskId: 'task-new' },
    [session],
    false,
  ],
  [
    'confirms a title change from a matching session snapshot',
    { type: 'title', sessionId: session.id, title: 'A clearer title' },
    [{ ...session, title: 'A clearer title' }],
    true,
  ],
  [
    'keeps a title change pending for the old session snapshot',
    { type: 'title', sessionId: session.id, title: 'A clearer title' },
    [session],
    false,
  ],
  [
    'does not predict the generated title after clearing its manual override',
    { type: 'title', sessionId: session.id, title: '' },
    [session],
    false,
  ],
]

it.each(sessionListConfirmationCases)(
  '%s',
  (_name, operation, serverSessions, expected) => {
    expect(isSessionOperationConfirmed(operation, serverSessions)).toEqual(
      expected,
    )
  },
)

it('does not wait for state confirmation when clearing a manual title', () => {
  expect(
    requiresStateConfirmation({
      type: 'title',
      sessionId: session.id,
      title: '  ',
    }),
  ).toEqual(false)
})

it.each([
  {
    action: 'preview',
    operation: {
      type: 'command_preview',
      sessionId: session.id,
      messageText: 'Run `! printf safe`.',
      commands: ['printf safe'],
    },
  },
  {
    action: 'execute',
    operation: {
      type: 'command_execute',
      sessionId: session.id,
      messageText: 'Run `! printf safe`.',
      commands: ['printf safe'],
    },
  },
  {
    action: 'save',
    operation: {
      type: 'review_document_save',
      sessionId: session.id,
      id: 'fixture-review',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
    },
  },
  {
    action: 'finish',
    operation: {
      type: 'review_document_finish',
      sessionId: session.id,
      id: 'fixture-review',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
    },
  },
] satisfies { action: string; operation: SessionOperation }[])(
  'does not wait for state confirmation for $action',
  ({ operation }) => {
    expect(requiresStateConfirmation(operation)).toEqual(false)
  },
)

it('does not wait for server state after rewind', () => {
  expect(
    requiresStateConfirmation({
      type: 'rewind',
      sessionId: session.id,
      messageId: 'fixture-user-message',
      mode: 'conversation',
    }),
  ).toEqual(false)
})

const permissionMessage: Extract<ChatItem, { type: 'permission' }> = {
  type: 'permission',
  requestId: 'permission-request',
  tool: 'Shell',
  description: 'Run the formatter.',
}
const allowedActions: SessionActions = {
  message: true,
  answerQuestion: true,
  permission: { approve: true, reject: true },
  interrupt: true,
  openInTerminal: true,
  rewind: false,
}

const answeredQuestion: Extract<ChatItem, { type: 'question' }> = {
  ...questionMessage,
  questions: questionMessage.questions.map((question) => ({
    ...question,
    answer: 'Option A',
  })),
}
const approvedDecision = { ...decisionMessage, review: 'ok' as const }
const changedDecision = { ...decisionMessage, review: 'change' as const }
const answeredProposal = {
  ...proposalMessage,
  answer: { choice: 0, comment: 'Use the first option.' },
  resolved: true,
}
const completedTodo = {
  ...todoMessage,
  completed: true,
  resolution: 'completed' as const,
  comment: 'Done.',
}
const rejectedTodo = {
  ...todoMessage,
  completed: true,
  resolution: 'rejected' as const,
  comment: 'Not needed',
}
const retriedPermission = { ...deniedMessage, resolved: true }

const conversationConfirmationCases: [
  string,
  SessionOperation,
  ConversationResponse,
  SessionActions | null,
  boolean,
][] = [
  [
    'keeps a question answer pending on the old answer',
    {
      type: 'answer',
      sessionId: session.id,
      requestId: 'question-request',
      answers: { 'Choose an option': 'Option A' },
    },
    conversation,
    allowedActions,
    false,
  ],
  [
    'confirms a question answer from the updated answer',
    {
      type: 'answer',
      sessionId: session.id,
      requestId: 'question-request',
      answers: { 'Choose an option': 'Option A' },
    },
    {
      ...conversation,
      messages: [answeredQuestion, ...conversation.messages.slice(1)],
    },
    allowedActions,
    true,
  ],
  [
    'confirms a skipped question after it disappears',
    {
      type: 'answer',
      sessionId: session.id,
      requestId: 'question-request',
      skip: true,
    },
    { ...conversation, messages: conversation.messages.slice(1) },
    allowedActions,
    true,
  ],
  [
    'keeps a decision review pending on the old decision',
    { type: 'decision_ok', sessionId: session.id, seq: decisionMessage.seq },
    conversation,
    allowedActions,
    false,
  ],
  [
    'confirms a decision review from the updated decision',
    { type: 'decision_ok', sessionId: session.id, seq: decisionMessage.seq },
    {
      ...conversation,
      messages: [
        questionMessage,
        approvedDecision,
        ...conversation.messages.slice(2),
      ],
      decisions: [approvedDecision],
    },
    allowedActions,
    true,
  ],
  [
    'confirms a decision change from the updated decision',
    {
      type: 'decision_change',
      sessionId: session.id,
      seq: decisionMessage.seq,
      comment: 'Change the retry limit.',
    },
    {
      ...conversation,
      messages: [
        questionMessage,
        changedDecision,
        ...conversation.messages.slice(2),
      ],
      decisions: [changedDecision],
    },
    allowedActions,
    true,
  ],
  [
    'keeps a proposal answer pending while the proposal is unresolved',
    {
      type: 'proposal_answer',
      sessionId: session.id,
      seq: proposalMessage.seq,
      choice: 0,
      comment: 'Use the first option.',
    },
    conversation,
    allowedActions,
    false,
  ],
  [
    'confirms a proposal answer from the resolved proposal',
    {
      type: 'proposal_answer',
      sessionId: session.id,
      seq: proposalMessage.seq,
      choice: 0,
      comment: 'Use the first option.',
    },
    {
      ...conversation,
      messages: [
        questionMessage,
        decisionMessage,
        answeredProposal,
        ...conversation.messages.slice(3),
      ],
      hasUnansweredProposal: false,
    },
    allowedActions,
    true,
  ],
  [
    'keeps a todo completion pending while the todo is open',
    {
      type: 'todo_resolve',
      sessionId: session.id,
      seq: todoMessage.seq,
      resolution: 'completed',
      comment: 'Done.',
    },
    conversation,
    allowedActions,
    false,
  ],
  [
    'confirms a todo completion from the completed todo',
    {
      type: 'todo_resolve',
      sessionId: session.id,
      seq: todoMessage.seq,
      resolution: 'completed',
      comment: 'Done.',
    },
    {
      ...conversation,
      messages: [
        ...conversation.messages.slice(0, 3),
        completedTodo,
        deniedMessage,
      ],
    },
    allowedActions,
    true,
  ],
  [
    'keeps a todo rejection pending while the todo is open',
    {
      type: 'todo_resolve',
      sessionId: session.id,
      seq: todoMessage.seq,
      resolution: 'rejected',
      comment: 'Not needed',
    },
    conversation,
    allowedActions,
    false,
  ],
  [
    'confirms a todo rejection from the rejected todo',
    {
      type: 'todo_resolve',
      sessionId: session.id,
      seq: todoMessage.seq,
      resolution: 'rejected',
      comment: 'Not needed',
    },
    {
      ...conversation,
      messages: [
        ...conversation.messages.slice(0, 3),
        rejectedTodo,
        deniedMessage,
      ],
    },
    allowedActions,
    true,
  ],
  [
    'keeps permission approval pending while permission is available',
    {
      type: 'permission',
      sessionId: session.id,
      action: 'approve',
      requestId: 'permission-request',
    },
    {
      ...conversation,
      messages: [...conversation.messages, permissionMessage],
    },
    allowedActions,
    false,
  ],
  [
    'confirms permission approval after the action is disabled',
    {
      type: 'permission',
      sessionId: session.id,
      action: 'approve',
      requestId: 'permission-request',
    },
    {
      ...conversation,
      messages: [...conversation.messages, permissionMessage],
    },
    { ...allowedActions, permission: { approve: false, reject: true } },
    true,
  ],
  [
    'keeps a denied action retry pending while it is unresolved',
    { type: 'permission_retry', sessionId: session.id, seq: deniedMessage.seq },
    conversation,
    allowedActions,
    false,
  ],
  [
    'confirms a denied action retry from the resolved item',
    { type: 'permission_retry', sessionId: session.id, seq: deniedMessage.seq },
    {
      ...conversation,
      messages: [...conversation.messages.slice(0, 4), retriedPermission],
    },
    allowedActions,
    true,
  ],
]

it.each(conversationConfirmationCases)(
  '%s',
  (_name, operation, serverConversation, actions, expected) => {
    expect(
      isConversationOperationConfirmed(serverConversation, operation, actions),
    ).toEqual(expected)
  },
)

it('does not confirm a second matching send from the first server message', () => {
  const firstOperation: SessionOperation = {
    type: 'message',
    sessionId: session.id,
    operationId: 'first-message',
    content: 'Repeated message',
    messageCountBefore: conversation.total,
  }
  const secondOperation: SessionOperation = {
    ...firstOperation,
    operationId: 'second-message',
    messageCountBefore: conversation.total + 1,
  }
  const serverConversation: ConversationResponse = {
    ...conversation,
    messages: [
      ...conversation.messages,
      { type: 'user', text: 'Repeated message' },
    ],
    total: conversation.total + 1,
  }

  const confirmationSnapshot = () => [
    isConversationOperationConfirmed(serverConversation, firstOperation, null),
    isConversationOperationConfirmed(serverConversation, secondOperation, null),
    sessionOperationKey(firstOperation),
    sessionOperationKey(secondOperation),
  ]
  expect(confirmationSnapshot()).toEqual([
    true,
    false,
    `${session.id}:message:first-message`,
    `${session.id}:message:second-message`,
  ])
})
