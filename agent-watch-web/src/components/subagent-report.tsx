import { MarkdownMessage } from '#components/markdown-message'
import { ToolSummary } from '#components/tool-summary'
import type { ChatItem } from '#types'

type SubagentReportItem = Extract<ChatItem, { type: 'subagent_report' }>

export function SubagentReport({
  message,
  defaultOpen = false,
}: {
  message: SubagentReportItem
  defaultOpen?: boolean
}) {
  return (
    <ToolSummary
      summary={message.description ?? message.agent_name}
      operations={[]}
      open={defaultOpen}
    >
      <MarkdownMessage>{message.text}</MarkdownMessage>
    </ToolSummary>
  )
}
