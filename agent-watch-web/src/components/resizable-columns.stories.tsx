import type { Meta, StoryObj } from '@storybook/react-vite'

import { ResizableColumns } from '#components/resizable-columns'

const meta = {
  component: ResizableColumns,
  args: {
    first: <div style={{ padding: '16px' }}>Session list</div>,
    second: <div style={{ padding: '16px' }}>Conversation</div>,
    storageKey: null,
    firstSize: 360,
    firstMinSize: 240,
    secondMinSize: 320,
    label: 'Resize session list and conversation',
  },
  decorators: [
    (Story) => (
      <div style={{ width: '900px', height: '560px' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ResizableColumns>

export default meta
type Story = StoryObj<typeof meta>

export const TwoColumnsHaveAnAdjustableBoundary: Story = {
  name: 'The session list and conversation have an adjustable boundary',
}
