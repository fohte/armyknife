import { expect, it } from 'vitest'

import { getSessionCloseCopy } from '#session-close-copy'
import type { Session } from '#types'

const session: Session = {
  id: 'sample-session',
  category: 'in_progress',
  activityStatus: 'working',
  isPaused: false,
  kind: null,
  title: 'Implement the sample workflow',
  preview: null,
  elapsedSeconds: 42,
  unread: false,
  task: null,
  repo: 'example/service',
  project: 'service',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

it.each([
  {
    name: 'working sessions',
    session,
    expected: {
      state: 'Working',
      title: 'Stop and close this session?',
      description:
        'It is working now. Closing stops the work in progress and ends the session.',
      confirmLabel: 'Stop and close',
    },
  },
  {
    name: 'question sessions',
    session: {
      ...session,
      category: 'needs_action',
      activityStatus: 'waiting',
      kind: 'Question',
    } satisfies Session,
    expected: {
      state: 'Question',
      title: 'Close without answering?',
      description:
        'It is waiting for your answer to a question. Closing ends the session, and the question stays unanswered.',
      confirmLabel: 'Close session',
    },
  },
  {
    name: 'proposal sessions',
    session: {
      ...session,
      category: 'needs_action',
      activityStatus: 'waiting',
      kind: 'Proposal',
    } satisfies Session,
    expected: {
      state: 'Proposal',
      title: 'Close without choosing?',
      description:
        'It is waiting for you to choose a proposal. Closing ends the session, and the proposal stays unchosen.',
      confirmLabel: 'Close session',
    },
  },
  {
    name: 'approval sessions',
    session: {
      ...session,
      category: 'needs_action',
      activityStatus: 'waiting',
      kind: 'Approve',
    } satisfies Session,
    expected: {
      state: 'Approve',
      title: 'Close without allowing?',
      description:
        'It is waiting for permission to run a command. Closing ends the session without running it.',
      confirmLabel: 'Close session',
    },
  },
  {
    name: 'review sessions',
    session: {
      ...session,
      category: 'needs_action',
      activityStatus: 'waiting',
      kind: 'Review',
    } satisfies Session,
    expected: {
      state: 'Review',
      title: 'Close before reviewing?',
      description:
        'It is waiting for your review. Closing ends the session; the pull request stays open.',
      confirmLabel: 'Close session',
    },
  },
  {
    name: 'plan sessions',
    session: {
      ...session,
      category: 'needs_action',
      activityStatus: 'waiting',
      kind: 'Plan',
    } satisfies Session,
    expected: {
      state: 'Plan',
      title: 'Close without confirming the plan?',
      description:
        'It is waiting for you to confirm its plan. Closing ends the session before any of the plan is carried out.',
      confirmLabel: 'Close session',
    },
  },
  {
    name: 'error sessions',
    session: {
      ...session,
      category: 'needs_action',
      activityStatus: 'waiting',
      kind: 'Error',
    } satisfies Session,
    expected: {
      state: 'Error',
      title: 'Close with the error unresolved?',
      description:
        'Its last run failed. Closing ends the session without fixing the error.',
      confirmLabel: 'Close session',
    },
  },
  {
    name: 'waiting sessions',
    session: { ...session, activityStatus: 'waiting' } satisfies Session,
    expected: {
      state: 'Waiting',
      title: 'Close this session?',
      description:
        'It is waiting for your next message. Closing ends the session.',
      confirmLabel: 'Close session',
    },
  },
  {
    name: 'Later sessions with an earlier request kind',
    session: {
      ...session,
      category: 'later',
      kind: 'Question',
    } satisfies Session,
    expected: {
      state: 'Later',
      title: 'Close this session?',
      description:
        'You set it aside for later. Closing ends the session, and it leaves Later.',
      confirmLabel: 'Close session',
    },
  },
])(
  'returns the state-specific confirmation copy for $name',
  ({ session, expected }) => {
    expect(getSessionCloseCopy(session)).toEqual(expected)
  },
)
