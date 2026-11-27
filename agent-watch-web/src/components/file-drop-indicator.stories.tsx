import type { Meta, StoryObj } from '@storybook/react-vite'

import { FileDropIndicator } from '#components/file-drop-indicator'

const meta = {
  component: FileDropIndicator,
} satisfies Meta<typeof FileDropIndicator>

export default meta
type Story = StoryObj<typeof meta>

export const ConversationDropTarget: Story = {
  name: 'The conversation pane shows the image drop prompt',
  args: { label: 'Drop images to attach', variant: 'conversation' },
  decorators: [
    (Story) => (
      <div style={{ position: 'relative', width: 420, height: 180 }}>
        <textarea
          aria-label="Comment"
          readOnly
          style={{
            boxSizing: 'border-box',
            height: '100%',
            width: '100%',
            padding: 10,
          }}
        />
        <Story />
      </div>
    ),
  ],
}

export const CardDropTarget: Story = {
  name: 'A comment field keeps the original file drop prompt',
  args: { label: 'Drop files to attach', variant: 'card' },
  decorators: [
    (Story) => (
      <div style={{ position: 'relative', width: 420, height: 92 }}>
        <textarea
          aria-label="Comment"
          readOnly
          style={{
            boxSizing: 'border-box',
            height: '100%',
            width: '100%',
            padding: 10,
          }}
        />
        <Story />
      </div>
    ),
  ],
}
