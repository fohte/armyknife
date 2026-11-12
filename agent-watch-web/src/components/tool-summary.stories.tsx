import type { Decorator, Meta, StoryObj } from '@storybook/react-vite'

import { ToolSummary } from '#components/tool-summary'

const conversationWidthDecorator: Decorator = (Story) => (
  <div
    style={{
      boxSizing: 'border-box',
      display: 'flex',
      width: '600px',
      minWidth: '600px',
      maxWidth: '600px',
      height: '400px',
      flex: 'none',
      flexDirection: 'column',
      gap: '14px',
      overflowY: 'auto',
      padding: '24px',
    }}
  >
    <Story />
  </div>
)

const meta = {
  component: ToolSummary,
  args: {
    summary: 'Edit sample.rs  Read ×1  exec ×1  Agent ×1',
    operations: [
      { tool: 'Read', target: 'src/sample.rs', failed: false },
      {
        tool: 'exec',
        target: "printf '%s\\n' 'hello' | grep hello",
        failed: false,
      },
      {
        tool: 'Agent',
        target: 'Explore · Find fixture loaders',
        failed: false,
      },
      { tool: 'Edit', target: 'src/sample.rs', failed: false },
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
  name: 'the failed operation is highlighted in red',
  args: {
    summary: '3 actions · 1 failed',
    operations: [
      { tool: 'Read', target: 'src/sample.rs', failed: false },
      { tool: 'Edit', target: 'src/sample.rs', failed: false },
      { tool: 'exec', target: 'fictional failure command', failed: true },
    ],
  },
}

export const ExpandedOperationsOmitEmptyTargets: Story = {
  name: 'the expanded summary omits empty targets',
  args: {
    summary: '2 actions',
    operations: [
      { tool: 'Read', target: null, failed: false },
      { tool: 'Edit', target: '', failed: false },
    ],
    open: true,
  },
}

export const LongOperationPathFitsConversation: Story = {
  name: 'An expanded summary keeps a long operation path inside the conversation',
  args: {
    open: true,
    operations: [
      {
        tool: 'Read',
        target:
          'sample/workspace/verification/runs/retry-policy/output/collected/responses/fixtures/request-timing-summary.json',
        failed: false,
      },
    ],
  },
  decorators: [conversationWidthDecorator],
}

export const LongCollapsedOperationPathFitsConversation: Story = {
  name: 'A collapsed summary truncates a long operation path inside the conversation',
  args: {
    summary:
      'Read sample/workspace/verification/runs/retry-policy/output/collected/responses/fixtures/request-timing-summary.json',
    operations: [
      {
        tool: 'Read',
        target:
          'sample/workspace/verification/runs/retry-policy/output/collected/responses/fixtures/request-timing-summary.json',
        failed: false,
      },
    ],
  },
  decorators: [conversationWidthDecorator],
}
