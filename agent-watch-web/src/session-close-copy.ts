import type { Session } from '#types'

export type SessionCloseCopy = {
  state: string
  title: string
  description: string
  confirmLabel: string
}

export function getSessionCloseCopy(session: Session): SessionCloseCopy {
  if (
    session.category === 'in_progress' &&
    session.activityStatus === 'working'
  ) {
    const hasNoTerminalPane = session.canOpenInTerminal === false

    return {
      state: 'Working',
      title: hasNoTerminalPane
        ? 'Close without stopping the work?'
        : 'Stop and close this session?',
      description: hasNoTerminalPane
        ? 'This session has no terminal pane to stop. Closing marks it ended, but the work in progress may continue.'
        : 'It is working now. Closing stops the work in progress and ends the session.',
      confirmLabel: hasNoTerminalPane ? 'Close session' : 'Stop and close',
    }
  }

  if (session.category === 'later') {
    return {
      state: 'Later',
      title: 'Close this session?',
      description:
        'You set it aside for later. Closing ends the session, and it leaves Later.',
      confirmLabel: 'Close session',
    }
  }

  switch (session.kind) {
    case 'Question':
      return {
        state: 'Question',
        title: 'Close without answering?',
        description:
          'It is waiting for your answer to a question. Closing ends the session, and the question stays unanswered.',
        confirmLabel: 'Close session',
      }
    case 'Proposal':
      return {
        state: 'Proposal',
        title: 'Close without choosing?',
        description:
          'It is waiting for you to choose a proposal. Closing ends the session.',
        confirmLabel: 'Close session',
      }
    case 'Approve':
      return {
        state: 'Approve',
        title: 'Close without allowing?',
        description:
          'It is waiting for permission to run a command. Closing ends the session without running it.',
        confirmLabel: 'Close session',
      }
    case 'Review':
      return {
        state: 'Review',
        title: 'Close before reviewing?',
        description:
          'It is waiting for your review. Closing ends the session; the pull request stays open.',
        confirmLabel: 'Close session',
      }
    case 'Plan':
      return {
        state: 'Plan',
        title: 'Close without confirming the plan?',
        description:
          'It is waiting for you to confirm its plan. Closing ends the session before any of the plan is carried out.',
        confirmLabel: 'Close session',
      }
    case 'Error':
      return {
        state: 'Error',
        title: 'Close with the error unresolved?',
        description:
          'Its last run failed. Closing ends the session without fixing the error.',
        confirmLabel: 'Close session',
      }
    default:
      return {
        state: 'Waiting',
        title: 'Close this session?',
        description:
          'It is waiting for your next message. Closing ends the session.',
        confirmLabel: 'Close session',
      }
  }
}
