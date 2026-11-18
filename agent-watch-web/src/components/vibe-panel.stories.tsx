import type { Meta, StoryObj } from '@storybook/react-vite'

import { VibePanel } from '#components/vibe-panel'

const meta = {
  component: VibePanel,
  args: {
    mode: { mode: 'enabled' },
    repo: 'example/service',
    onClose: () => undefined,
    onChange: () => undefined,
  },
} satisfies Meta<typeof VibePanel>

export default meta
type Story = StoryObj<typeof meta>

export const Enabled: Story = {
  name: 'The enabled session can turn vibe mode off',
}

export const Inherited: Story = {
  name: 'The inherited session can enable vibe mode for itself',
  args: {
    mode: { mode: 'inherited', ancestorSessionId: 'sample-parent' },
  },
}

export const Disabled: Story = {
  name: 'The disabled session can turn vibe mode on',
  args: { mode: { mode: 'disabled' } },
}

export const Unavailable: Story = {
  name: 'The repository setting explains why vibe mode is unavailable',
  args: { mode: { mode: 'repo_disabled' } },
}

export const FailedChange: Story = {
  name: 'A rejected change shows the server reason',
  args: {
    mode: { mode: 'disabled' },
    error: 'Vibe mode is disabled by configuration for example/service.',
  },
}

export const InheritedOnMobile: Story = {
  name: 'The inherited session details fit in a mobile sheet',
  args: {
    mode: { mode: 'inherited', ancestorSessionId: 'sample-parent' },
    isMobile: true,
  },
  decorators: [
    (Story) => (
      <div style={{ position: 'relative', height: 500, background: '#0a0a0a' }}>
        <Story />
      </div>
    ),
  ],
}
