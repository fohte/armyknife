import type { Meta, StoryObj } from '@storybook/react-vite'

import { PeerMessage } from '#components/peer-message'
import type { ChatItem, SessionLinkTarget } from '#types'

const target: SessionLinkTarget = {
  id: 'linked-session',
  title: 'Build verification',
  task: null,
  repo: 'example/project',
}

const meta = {
  component: PeerMessage,
  args: {
    message: {
      type: 'peer',
      direction: 'from',
      session_id: target.id,
      content: [
        { type: 'text', text: 'Please review the fictional test output.' },
      ],
    } satisfies Extract<ChatItem, { type: 'peer' }>,
    onSelectSession: () => undefined,
    sessions: [target],
  },
  decorators: [
    (Story) => (
      <div
        style={{
          width: 672,
          padding: 24,
          background: 'var(--aw-bg-app)',
        }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof PeerMessage>

export default meta
type Story = StoryObj<typeof meta>

export const PeerMessageArrivesFromAnotherSession: Story = {
  name: 'A peer message shows its source on the left',
}

export const PeerMessageIsSentToAnotherSession: Story = {
  name: 'A peer message shows its destination on the right',
  args: {
    message: {
      type: 'peer',
      direction: 'to',
      session_id: target.id,
      content: [{ type: 'text', text: 'Please inspect the fictional branch.' }],
    },
  },
}

export const PeerMessageNamesAnUntrackedSession: Story = {
  name: 'A peer message shows an untracked session ID as plain text',
  args: {
    message: {
      type: 'peer',
      direction: 'to',
      session_id: 'fixture-untracked-session',
      content: [{ type: 'text', text: 'Please inspect the fictional branch.' }],
    },
    sessions: [],
  },
}

export const PeerMessageNamesArmyknife: Story = {
  name: 'A peer notification identifies armyknife as its source',
  args: {
    message: {
      type: 'peer',
      direction: 'from',
      session_id: 'unknown',
      content: [{ type: 'text', text: 'The fictional result is ready.' }],
    },
    sessions: [],
  },
}

export const PeerMessageShowsPreviousDay: Story = {
  name: 'A peer message shows its date when it was sent the previous day',
  args: {
    message: {
      type: 'peer',
      direction: 'from',
      session_id: target.id,
      timestamp: new Date(2030, 8, 11, 18, 58).toISOString(),
      content: [{ type: 'text', text: 'Please review the fictional changes.' }],
    } satisfies Extract<ChatItem, { type: 'peer' }>,
  },
  render: (args) => <PeerMessage {...args} now={new Date(2030, 8, 12, 12)} />,
}
