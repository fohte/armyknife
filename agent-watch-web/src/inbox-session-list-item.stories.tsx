import type { Meta, StoryObj } from '@storybook/react-vite'

import { InboxSessionListItem } from '#inbox-session-list-item'
import type { Session } from '#types'

const session: Session = {
  id: 'sample-session',
  category: 'needs_action',
  isPaused: false,
  kind: 'Review',
  title: 'Review the sample service changes',
  preview: 'Review the changes before the next release.',
  elapsedSeconds: 120,
  unread: true,
  task: null,
  repo: 'example/service',
  project: 'service',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const meta = {
  component: InboxSessionListItem,
  args: {
    session,
    selectedId: null,
    onSelect: () => undefined,
    isMobile: true,
    onLater: () => undefined,
    onCloseSession: () => undefined,
    onOpenActions: () => undefined,
  },
  decorators: [
    (Story) => (
      <div className="story-mobile-layout" data-layout="mobile">
        <ul className="session-list story-session-list">
          <Story />
        </ul>
      </div>
    ),
  ],
} satisfies Meta<typeof InboxSessionListItem>

export default meta
type Story = StoryObj<typeof meta>

export const PhoneRowIsSwiped: Story = {
  name: 'The phone row reveals Later and Close after a left swipe',
  args: { defaultSwiped: true },
}

export const PhoneRowIsPartiallySwiped: Story = {
  name: 'The phone row reveals actions within the swiped distance',
  args: { defaultDragOffset: -56 },
}

export const SelectedPhoneRowAtRest: Story = {
  name: 'The selected phone row hides Later and Close at rest',
  args: { selectedId: session.id },
}
