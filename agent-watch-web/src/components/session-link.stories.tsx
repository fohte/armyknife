import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionLink } from '#components/session-link'
import type { Session } from '#types'

const session: Session = {
  id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
  category: 'needs_action',
  isPaused: true,
  kind: 'Review',
  title: 'Review a sample change',
  preview: null,
  elapsedSeconds: 30,
  unread: false,
  task: {
    id: 'sample-task',
    number: 24,
    title: 'Inspect the example workflow',
    priority: 2,
  },
  repo: 'sample-org/example-service',
  project: 'Example service',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const meta = {
  component: SessionLink,
  args: {
    children: session.title,
    session,
  },
} satisfies Meta<typeof SessionLink>

export default meta
type Story = StoryObj<typeof meta>

export const CardIsOpen: Story = {
  args: { open: true },
  name: 'An open session link card shows state, task, and repository',
}

export const SessionHasNoTask: Story = {
  args: {
    open: true,
    session: { ...session, task: null },
  },
  name: 'The session card explains when no tq task is linked',
}
