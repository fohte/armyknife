import type { Meta, StoryObj } from '@storybook/react-vite'

import { ToolSummary } from '#components/tool-summary'

const meta = {
  component: ToolSummary,
  args: {
    summary: 'Edit sample.rs  Read ×1  exec ×1  Agent ×1',
    operations: [
      { tool: 'Read', target: 'src/sample.rs' },
      { tool: 'exec', target: "printf '%s\\n' 'hello' | grep hello" },
      { tool: 'Agent', target: 'Explore · Find fixture loaders' },
      { tool: 'Edit', target: 'src/sample.rs' },
    ],
    open: false,
  },
} satisfies Meta<typeof ToolSummary>

export default meta
type Story = StoryObj<typeof meta>

export const CollapsedSummaryShowsActivity: Story = {
  name: 'the collapsed summary shows activity text and a chevron',
}

export const ExpandedSummaryShowsToolTargets: Story = {
  name: 'the expanded summary shows each tool and its target',
  args: { open: true },
}

export const FailedSummaryHighlightsFailures: Story = {
  name: 'the failed count is highlighted in red',
  args: {
    summary: '3 actions · 2 failed',
    operations: [
      { tool: 'Read', target: 'src/sample.rs' },
      { tool: 'Edit', target: 'src/sample.rs' },
      { tool: 'exec', target: 'cargo test', failed: true },
    ],
  },
}

export const ExpandedOperationsOmitEmptyTargets: Story = {
  name: 'the expanded summary omits empty targets',
  args: {
    summary: '2 actions',
    operations: [
      { tool: 'Read', target: null },
      { tool: 'Edit', target: '' },
    ],
    open: true,
  },
}
