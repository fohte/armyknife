import '#components/tool-summary.css'

import { useState } from 'react'

import { MarkdownMessage } from '#components/markdown-message'
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
  const [isOpen, setIsOpen] = useState(defaultOpen)

  return (
    <details
      className="chat-tool-summary"
      open={isOpen}
      onToggle={(event) => {
        setIsOpen(event.currentTarget.open)
      }}
    >
      <summary>{message.description ?? message.agent_name}</summary>
      <MarkdownMessage onSelectSession={onSelectSession} sessions={sessions}>
        {message.text}
      </MarkdownMessage>
    </details>
  )
}
