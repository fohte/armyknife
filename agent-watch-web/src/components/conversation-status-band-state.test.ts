import { expect, it } from 'vitest'

import type {
  EditorReviewDocumentState,
  ReviewDocumentSummary,
} from '#components/conversation-pane-response'
import {
  type ConversationStatusBandState,
  getConversationStatusBandState,
} from '#components/conversation-status-band-state'
import type { BackgroundTask, ChatItem, Session } from '#types'

const session: Session = {
  id: 'fixture-session',
  category: 'in_progress',
  isPaused: false,
  kind: null,
  title: 'Refresh the sample service',
  preview: null,
  elapsedSeconds: 31,
  unread: false,
  task: {
    id: 'fixture-task',
    number: 8,
    title: 'Refresh the sample service',
    priority: 2,
  },
  repo: 'sample-org/sample-repo',
  project: 'sample service',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const backgroundTasks: readonly BackgroundTask[] = [
  {
    id: 'fixture-background-task',
    description: 'Check the sample build',
    elapsedSeconds: 42,
    kind: 'background_task',
  },
]

const question: ChatItem = {
  type: 'question',
  requestId: 'fixture-question-request',
  questions: [
    {
      header: 'Retry policy',
      question: 'Which retry interval should be used?',
      choices: [{ label: 'Short' }, { label: 'Long' }],
      multi_select: false,
      answer: null,
    },
  ],
}

const permission: ChatItem = {
  type: 'permission',
  requestId: 'fixture-permission-request',
  tool: 'exec',
  description: 'Run the sample check',
  command: 'cargo test --package sample',
}

const editorIssueComment: ReviewDocumentSummary = {
  id: 'fixture-review-document',
  kind: 'issue_comment',
  target: { issueNumber: 1234 },
  isEditor: true,
  canSave: true,
  canFinish: true,
}
const editorIssueCommentState: EditorReviewDocumentState = {
  status: 'ready',
  summary: editorIssueComment,
}

type StatusCase = {
  name: string
  input: {
    session: Session
    messages?: readonly ChatItem[]
    backgroundTasks?: readonly BackgroundTask[]
    editorReviewDocument?: EditorReviewDocumentState | null
  }
  expected: ConversationStatusBandState | null
}

const statusCases: StatusCase[] = [
  {
    name: 'working',
    input: { session: { ...session, activityStatus: 'working' as const } },
    expected: { kind: 'working', detail: null },
  },
  {
    name: 'compacting',
    input: {
      session: {
        ...session,
        isCompacting: true,
        activityStatus: 'working' as const,
      },
    },
    expected: { kind: 'compacting', detail: null },
  },
  {
    name: 'idle compacting',
    input: {
      session: {
        ...session,
        isCompacting: true,
        activityStatus: 'stopped' as const,
      },
    },
    expected: { kind: 'compacting', detail: null },
  },
  {
    name: 'background tasks',
    input: {
      session: {
        ...session,
        activityStatus: 'stopped' as const,
        waitingForBackgroundTasks: true,
      },
      backgroundTasks,
    },
    expected: { kind: 'background-tasks', detail: 'Check the sample build' },
  },
  {
    name: 'background tasks while working',
    input: {
      session: {
        ...session,
        activityStatus: 'working' as const,
        waitingForBackgroundTasks: true,
      },
      backgroundTasks,
    },
    expected: { kind: 'background-tasks', detail: 'Check the sample build' },
  },
  {
    name: 'crit review',
    input: {
      session: {
        ...session,
        category: 'needs_action',
        activityStatus: 'waiting' as const,
        isWaitingForCritReview: true,
        kind: 'Review',
        pullRequest: { number: 742, state: 'Open' },
        critUrl: 'https://crit.example.test/review/fixture',
      },
    },
    expected: { kind: 'crit-review', detail: 'crit · PR #742' },
  },
  {
    name: 'one draft review',
    input: {
      session: {
        ...session,
        reviewDocumentId: editorIssueComment.id,
        reviewDocumentCount: 1,
      },
      editorReviewDocument: editorIssueCommentState,
    },
    expected: {
      kind: 'draft-review',
      detail: '1 draft · Issue comment #1234 is in the editor',
    },
  },
  {
    name: 'multiple drafts and Crit review',
    input: {
      session: {
        ...session,
        category: 'needs_action',
        activityStatus: 'waiting',
        isWaitingForCritReview: true,
        kind: 'Review',
        pullRequest: { number: 742, state: 'Open' },
        critUrl: 'https://crit.example.test/review/fixture',
        reviewDocumentId: editorIssueComment.id,
        reviewDocumentCount: 4,
      },
      editorReviewDocument: editorIssueCommentState,
    },
    expected: {
      kind: 'draft-review',
      detail:
        '4 drafts · Issue comment #1234 is in the editor · crit · PR #742',
    },
  },
  {
    name: 'draft details unavailable',
    input: {
      session: {
        ...session,
        reviewDocumentId: editorIssueComment.id,
        reviewDocumentCount: 1,
      },
      editorReviewDocument: { status: 'unavailable' },
    },
    expected: {
      kind: 'draft-review',
      detail: '1 draft · Draft details are unavailable',
    },
  },
  {
    name: 'crit review with pull requests',
    input: {
      session: {
        ...session,
        category: 'needs_action',
        activityStatus: 'waiting' as const,
        isWaitingForCritReview: true,
        kind: 'Review',
        pullRequests: [
          { number: 864, state: 'Open' },
          { number: 865, state: 'Open' },
        ],
        critUrl: 'https://crit.example.test/review/fixture',
      },
    },
    expected: { kind: 'crit-review', detail: 'crit · PR #864' },
  },
  ...(['Todo', 'Proposal'] as const).map((kind): StatusCase => ({
    name: `crit review with ${kind} kind`,
    input: {
      session: {
        ...session,
        category: 'needs_action' as const,
        activityStatus: 'waiting' as const,
        isWaitingForCritReview: true,
        kind,
        critUrl: 'https://crit.example.test/review/fixture',
      },
    },
    expected: { kind: 'crit-review', detail: 'crit' },
  })),
  {
    name: 'registered Crit review',
    input: {
      session: {
        ...session,
        category: 'needs_action',
        activityStatus: 'stopped' as const,
        isWaitingForCritReview: true,
        kind: 'Review',
        webUis: [
          {
            name: 'crit',
            url: 'https://crit.example.test/review/fixture',
          },
        ],
      },
    },
    expected: { kind: 'crit-review', detail: 'crit' },
  },
  {
    name: 'crit already replied while todo remains',
    input: {
      session: {
        ...session,
        category: 'needs_action',
        activityStatus: 'working' as const,
        isWaitingForCritReview: false,
        kind: 'Todo',
        preview: 'Check the sample page',
        critUrl: 'https://crit.example.test/review/fixture',
      },
    },
    expected: { kind: 'waiting', detail: 'Check the sample page' },
  },
  {
    name: 'empty Crit URL',
    input: {
      session: {
        ...session,
        category: 'needs_action',
        activityStatus: 'stopped' as const,
        kind: 'Review',
        critUrl: '',
      },
    },
    expected: { kind: 'waiting', detail: null },
  },
  {
    name: 'question',
    input: {
      session: { ...session, category: 'needs_action', kind: 'Question' },
      messages: [question],
    },
    expected: {
      kind: 'question',
      detail: 'Which retry interval should be used?',
    },
  },
  {
    name: 'question with a draft',
    input: {
      session: {
        ...session,
        category: 'needs_action',
        kind: 'Question',
        reviewDocumentId: editorIssueComment.id,
        reviewDocumentCount: 1,
      },
      messages: [question],
    },
    expected: {
      kind: 'question',
      detail: 'Which retry interval should be used?',
    },
  },
  {
    name: 'permission',
    input: {
      session: { ...session, category: 'needs_action', kind: 'Approve' },
      messages: [permission],
    },
    expected: {
      kind: 'permission',
      detail: 'Bash  cargo test --package sample',
    },
  },
  {
    name: 'permission with a draft',
    input: {
      session: {
        ...session,
        category: 'needs_action',
        kind: 'Approve',
        reviewDocumentId: editorIssueComment.id,
        reviewDocumentCount: 1,
      },
      messages: [permission],
    },
    expected: {
      kind: 'permission',
      detail: 'Bash  cargo test --package sample',
    },
  },
  {
    name: 'other wait',
    input: {
      session: {
        ...session,
        activityStatus: 'waiting' as const,
        waitingReason: 'Waiting for a sample check',
      },
    },
    expected: { kind: 'waiting', detail: 'Waiting for a sample check' },
  },
  {
    name: 'stopped',
    input: {
      session: { ...session, activityStatus: 'stopped' as const },
    },
    expected: null,
  },
]

it.each(statusCases)(
  'maps $name to its status band state',
  ({ input, expected }) => {
    expect(
      getConversationStatusBandState({
        session: input.session,
        messages: input.messages ?? [],
        backgroundTasks: input.backgroundTasks ?? [],
        editorReviewDocument: input.editorReviewDocument ?? null,
      }),
    ).toEqual(expected)
  },
)

it('shows an active tool while the session is working and marked as compacting', () => {
  expect(
    getConversationStatusBandState({
      session: {
        ...session,
        activityStatus: 'working',
        isCompacting: true,
      },
      messages: [],
      backgroundTasks: [],
      activeTool: { tool: 'Bash', target: 'cargo test --package sample' },
    }),
  ).toEqual({ kind: 'working', detail: null })
})

it('keeps draft review primary while an active tool is marked as compacting', () => {
  expect(
    getConversationStatusBandState({
      session: {
        ...session,
        activityStatus: 'working',
        isCompacting: true,
        reviewDocumentId: editorIssueComment.id,
        reviewDocumentCount: 1,
      },
      messages: [],
      backgroundTasks: [],
      activeTool: { tool: 'Bash', target: 'cargo test --package sample' },
      editorReviewDocument: editorIssueCommentState,
    }),
  ).toEqual({
    kind: 'draft-review',
    detail: '1 draft · Issue comment #1234 is in the editor',
  })
})

it('keeps Crit review primary while background tasks continue', () => {
  expect(
    getConversationStatusBandState({
      session: {
        ...session,
        category: 'needs_action',
        activityStatus: 'waiting',
        isWaitingForCritReview: true,
        kind: 'Review',
        pullRequest: { number: 742, state: 'Open' },
        critUrl: 'https://crit.example.test/review/fixture',
      },
      messages: [],
      backgroundTasks,
    }),
  ).toEqual({ kind: 'crit-review', detail: 'crit · PR #742' })
})

it('shows a pending question before the Crit review', () => {
  expect(
    getConversationStatusBandState({
      session: {
        ...session,
        category: 'needs_action',
        isWaitingForCritReview: true,
        kind: 'Todo',
        critUrl: 'https://crit.example.test/review/fixture',
      },
      messages: [question],
      backgroundTasks: [],
    }),
  ).toEqual({
    kind: 'question',
    detail: 'Which retry interval should be used?',
  })
})

it('shows a pending permission before the Crit review', () => {
  expect(
    getConversationStatusBandState({
      session: {
        ...session,
        category: 'needs_action',
        isWaitingForCritReview: true,
        kind: 'Todo',
        critUrl: 'https://crit.example.test/review/fixture',
      },
      messages: [
        {
          type: 'permission',
          requestId: 'fixture-permission-request',
          tool: 'exec',
          description: 'Run the sample check',
          command: 'cargo test --package sample',
        } satisfies ChatItem,
      ],
      backgroundTasks: [],
    }),
  ).toEqual({ kind: 'permission', detail: 'Bash  cargo test --package sample' })
})

it('ignores stale unanswered questions while the session waits for a Crit review', () => {
  expect(
    getConversationStatusBandState({
      session: {
        ...session,
        category: 'needs_action',
        isWaitingForCritReview: true,
        kind: 'Review',
        critUrl: 'https://crit.example.test/review/fixture',
      },
      messages: [{ ...question, cancelled: true }],
      backgroundTasks: [],
    }),
  ).toEqual({ kind: 'crit-review', detail: 'crit' })
})
