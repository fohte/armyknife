import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionLink } from '#components/session-link'
import type { SessionLinkTarget } from '#types'

const session: SessionLinkTarget = {
  id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
  engine: 'claude',
  workType: 'PM',
  kind: 'Review',
  title: 'Review a sample change',
  preview: 'Please review the sample design.',
  elapsedSeconds: 120,
  task: {
    id: 'sample-task',
    number: 24,
    title: 'Inspect the example workflow',
    priority: 2,
  },
  linkedTaskCount: 3,
  repo: 'sample-org/example-service',
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
      linkedTaskCount: 0,
      repo: '',
    },
  },
  name: 'The card hides activity details when the session has none',
}
