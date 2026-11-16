import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionHeading } from '#components/session-heading'

const meta = {
  component: SessionHeading,
  render: (args) => (
    <div className="conversation-header-copy">
      <SessionHeading {...args} />
    </div>
  ),
  args: {
    title: 'Investigate a failing build',
    sessionId: 'sample-session-7f3a',
  },
} satisfies Meta<typeof SessionHeading>

export default meta
type Story = StoryObj<typeof meta>

export const SessionTitleAndCopyControl: Story = {
  name: 'The session title stays beside its copy button',
}

export const SessionWithoutCopyControl: Story = {
  args: { title: 'Investigate a failing build' },
  render: ({ title }) => (
    <div className="conversation-header-copy">
      <SessionHeading title={title} />
    </div>
  ),
  name: 'The session title appears without a copy button',
}
