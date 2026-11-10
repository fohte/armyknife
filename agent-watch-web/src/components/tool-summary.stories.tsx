import type { Meta, StoryObj } from '@storybook/react-vite'

import { ToolSummary } from '#components/tool-summary'

const meta = {
  component: ToolSummary,
  args: {
    summary: '4 actions · 2 files edited',
    operations: [
      { tool: 'Read', target: 'src/sample.rs' },
      { tool: 'exec', target: "printf '%s\\n' 'hello' | grep hello" },
      { tool: 'Agent', target: 'Explore · Find fixture loaders' },
      { tool: 'Edit', target: 'src/sample.rs' },
    ],
    open: true,
  },
} satisfies Meta<typeof ToolSummary>

export default meta
type Story = StoryObj<typeof meta>

export const ExpandedSummaryShowsToolTargets: Story = {
  name: 'the expanded summary shows each tool and its target',
}
