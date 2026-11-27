import type { Meta, StoryObj } from '@storybook/react-vite'

import { FileDropIndicator } from '#components/file-drop-indicator'

const meta = {
  component: FileDropIndicator,
  args: { label: 'Drop images to attach' },
} satisfies Meta<typeof FileDropIndicator>

export default meta
type Story = StoryObj<typeof meta>

export const VisibleOverAFileDropTarget: Story = {
  name: 'A file drop target shows where files can be attached',
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
