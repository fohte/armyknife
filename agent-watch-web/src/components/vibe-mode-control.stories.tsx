import type { Meta, StoryObj } from '@storybook/react-vite'

import { VibeModeControl } from '#components/vibe-mode-control'

const meta = {
  component: VibeModeControl,
  args: {
    sessionId: 'sample-session',
    repo: 'example/service',
    mode: { mode: 'enabled' },
  },
} satisfies Meta<typeof VibeModeControl>

export default meta
type Story = StoryObj<typeof meta>

export const Enabled: Story = {
  name: 'The header identifies an enabled session',
}

export const Inherited: Story = {
  name: 'The header identifies an inherited session',
  args: {
    mode: { mode: 'inherited', ancestorSessionId: 'sample-parent' },
  },
}

export const Disabled: Story = {
  name: 'The header identifies a disabled session',
  args: { mode: { mode: 'disabled' } },
}

export const Unavailable: Story = {
  name: 'The header identifies a repository that disallows vibe mode',
  args: { mode: { mode: 'repo_disabled' } },
}

export const InheritedSheetIsOpen: Story = {
  name: 'The mobile sheet shows the inherited state',
  args: {
    mode: { mode: 'inherited', ancestorSessionId: 'sample-parent' },
    isMobile: true,
    open: true,
  },
}
