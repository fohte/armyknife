import type { Meta, StoryObj } from '@storybook/react-vite'

import { ToolSummary } from '#components/tool-summary'

const meta = {
  component: ToolSummary,
  args: {
    summary: '3 actions · 2 files edited',
    operations: [
      { tool: 'Read', target: 'src/commands/agent/watch.rs' },
      { tool: 'exec', target: 'cargo test -p armyknife agent::watch' },
      { tool: 'Edit', target: 'src/commands/agent/watch.rs' },
    ],
    open: true,
  },
} satisfies Meta<typeof ToolSummary>

export default meta
type Story = StoryObj<typeof meta>

export const ExpandedSummaryShowsToolTargets: Story = {
  name: 'the expanded summary shows each tool and its target',
}
