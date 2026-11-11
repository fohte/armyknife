import type { Meta, StoryObj } from '@storybook/react-vite'

import { SubagentReport } from '#components/subagent-report'
import type { ChatItem } from '#types'

const meta = {
  component: SubagentReport,
  args: {
    message: {
      type: 'subagent_report',
      agent_id: 'fixture-agent-id',
      agent_name: 'general-purpose',
      description: 'Review the fictional feature',
      text: '## Result\n\nThe requested change is complete.',
    } satisfies Extract<ChatItem, { type: 'subagent_report' }>,
    onSelectSession: () => undefined,
    sessions: [],
  },
} satisfies Meta<typeof SubagentReport>

export default meta
type Story = StoryObj<typeof meta>

export const CollapsedReportShowsItsTask: Story = {
  name: 'A collapsed report shows the task description',
}

export const ExpandedReportShowsMarkdown: Story = {
  name: 'An expanded report shows its markdown body',
  args: { defaultOpen: true },
}
