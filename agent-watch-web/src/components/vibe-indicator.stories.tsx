import type { Meta, StoryObj } from '@storybook/react-vite'

import { VibeIndicator } from '#components/vibe-indicator'

const meta = {
  component: VibeIndicator,
  args: { mode: { mode: 'enabled' } },
  decorators: [
    (Story) => (
      <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
        <span>Session title</span>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof VibeIndicator>

export default meta
type Story = StoryObj<typeof meta>

export const Enabled: Story = {
  name: 'The enabled session shows a filled lightning bolt',
}

export const Inherited: Story = {
  name: 'The inherited session shows an outlined lightning bolt',
  args: { mode: { mode: 'inherited', ancestorSessionId: 'sample-parent' } },
}

export const Disabled: Story = {
  name: 'The disabled session has no lightning bolt',
  args: { mode: { mode: 'disabled' } },
}
