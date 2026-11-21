import type { Meta, StoryObj } from '@storybook/react-vite'

import { InboxSaveViewSheet } from '#components/inbox-save-view-sheet'

const meta = {
  component: InboxSaveViewSheet,
  args: {
    query: 'project:sample-service kind:question,review',
    name: 'Sample service reviews',
    onNameChange: () => undefined,
    onSave: () => undefined,
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
} satisfies Meta<typeof InboxSaveViewSheet>

export default meta
type Story = StoryObj<typeof meta>

export const SaveViewSheetIsOpen: Story = {
  name: 'The sheet names and saves the active filter view',
}
