import type { Meta, StoryObj } from '@storybook/react-vite'

import { InboxViewDialogLayers } from '#components/inbox-view-dialog-layers'
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
  component: InboxViewDialogLayers,
  args: {
    sessions: [session],
    mobile: true,
    filterDialogOpen: false,
    draftQuery: 'kind:review',
    onDraftQueryChange: () => undefined,
    onApplyFilter: () => undefined,
    onCloseFilter: () => undefined,
    actionSheetSession: null,
    onLater: () => undefined,
    onCloseSession: () => undefined,
    onDismissActions: () => undefined,
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
} satisfies Meta<typeof InboxViewDialogLayers>

export default meta
type Story = StoryObj<typeof meta>

export const IPhoneFilterIsFullScreen: Story = {
  name: 'The iPhone filter fills the screen above its matching sessions action',
  args: { filterDialogOpen: true },
  parameters: {
    overflowCheck: { ignoreSelectors: ['.inbox-filter-fields'] },
  },
}

export const IPhoneRowActionsAreOpen: Story = {
  name: 'The long press sheet offers Later and Close session actions',
  args: { actionSheetSession: session },
}
