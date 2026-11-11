import { MarkdownMessage } from '#components/markdown-message'
import { ToolSummary } from '#components/tool-summary'
import type { ChatItem, SessionLinkTarget } from '#types'

type SubagentReportItem = Extract<ChatItem, { type: 'subagent_report' }>

export function SubagentReport({
  message,
  onSelectSession,
  sessions,
  defaultOpen = false,
}: {
  message: SubagentReportItem
  onSelectSession: (id: string) => void
  sessions: readonly SessionLinkTarget[]
  defaultOpen?: boolean
}) {
  return (
    <ToolSummary
      summary={message.description ?? message.agent_name}
      operations={[]}
      open={defaultOpen}
    >
      <MarkdownMessage onSelectSession={onSelectSession} sessions={sessions}>
        {message.text}
      </MarkdownMessage>
    </ToolSummary>
  )
}
