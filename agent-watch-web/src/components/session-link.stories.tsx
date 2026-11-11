import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionLink } from '#components/session-link'
import type { Session } from '#types'

const session: Session = {
  id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
  category: 'needs_action',
  isPaused: false,
  engine: 'Claude',
  workType: 'PM',
  kind: 'Review',
  title: 'Review a sample change',
  preview: 'Please review the sample design.',
  elapsedSeconds: 120,
  unread: false,
  task: {
    id: 'sample-task',
    number: 24,
    title: 'Inspect the example workflow',
    priority: 2,
  },
  tasks: [
    {
      id: 'sample-task',
      number: 24,
      title: 'Inspect the example workflow',
      priority: 2,
    },
    {
      id: 'sample-task-extra',
      number: 25,
      title: 'Improve the example workflow',
      priority: 3,
    },
    {
      id: 'sample-task-another',
      number: 26,
      title: 'Document the example workflow',
      priority: 4,
    },
  ],
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
  name: 'The open card shows session activity and linked tasks',
}

export const SessionHasNoTask: Story = {
  args: {
    open: true,
    session: {
      ...session,
      engine: undefined,
      workType: null,
      kind: null,
      preview: null,
      elapsedSeconds: undefined,
      task: null,
      repo: '',
    },
  },
  name: 'The card hides activity details when the session has none',
}
