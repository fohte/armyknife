import { expect, it } from 'vitest'

import { getConversationStatusBandState } from '#components/conversation-status-band-state'
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

it.each([
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
        kind: 'Review',
        critUrl: 'https://crit.example.test/review/fixture',
      },
    },
    expected: { kind: 'crit-review', detail: 'Refresh the sample service' },
  },
  {
    name: 'registered Crit review',
    input: {
      session: {
        ...session,
        category: 'needs_action',
        activityStatus: 'stopped' as const,
        kind: 'Review',
        webUis: [
          {
            name: 'crit',
            url: 'https://crit.example.test/review/fixture',
          },
        ],
      },
    },
    expected: { kind: 'crit-review', detail: 'Refresh the sample service' },
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
    name: 'permission',
    input: {
      session: { ...session, category: 'needs_action', kind: 'Approve' },
      messages: [
        {
          type: 'permission',
          requestId: 'fixture-permission-request',
          tool: 'exec',
          description: 'Run the sample check',
          command: 'cargo test --package sample',
        } satisfies ChatItem,
      ],
    },
    expected: { kind: 'permission', detail: 'cargo test --package sample' },
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
])('maps $name to its status band state', ({ input, expected }) => {
  expect(
    getConversationStatusBandState({
      session: input.session,
      messages: input.messages ?? [],
      backgroundTasks: input.backgroundTasks ?? [],
    }),
  ).toEqual(expected)
})

it('keeps Crit review primary while background tasks continue', () => {
  expect(
    getConversationStatusBandState({
      session: {
        ...session,
        category: 'needs_action',
        activityStatus: 'waiting',
        kind: 'Review',
        critUrl: 'https://crit.example.test/review/fixture',
      },
      messages: [],
      backgroundTasks,
    }),
  ).toEqual({ kind: 'crit-review', detail: 'Refresh the sample service' })
})
