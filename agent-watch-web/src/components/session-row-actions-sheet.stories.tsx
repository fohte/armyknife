import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionRowActionsSheet } from '#components/session-row-actions-sheet'
import type { Session } from '#types'

const session: Session = {
  id: 'sample-session',
  category: 'needs_action',
  isPaused: false,
  kind: 'Review',
  title: 'Review the sample service changes',
  preview: null,
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
  component: SessionRowActionsSheet,
  args: {
    session,
    onLater: () => undefined,
    onCloseSession: () => undefined,
    onCancel: () => undefined,
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        data-layout="mobile"
        style={{ width: '390px', height: '844px', position: 'relative' }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SessionRowActionsSheet>

export default meta
type Story = StoryObj<typeof meta>

export const SessionActionsAreOpen: Story = {
  name: 'The long press sheet offers Later and Close session actions',
}
