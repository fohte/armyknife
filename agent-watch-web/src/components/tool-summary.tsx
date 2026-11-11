import '#components/tool-summary.css'

import { type ReactNode, useState } from 'react'

import type { ToolOperation } from '#types'

type ToolSummaryProps = {
  summary: string
  operations: readonly ToolOperation[]
  children?: ReactNode
  open?: boolean
}

export function ToolSummary({
  summary,
  operations,
  children,
  open = false,
}: ToolSummaryProps) {
  const [isOpen, setIsOpen] = useState(open)
  const failedSuffix = summary.match(/· \d+ failed$/)?.[0]
  const summaryText =
    failedSuffix === undefined
      ? summary
      : summary.slice(0, -failedSuffix.length).trimEnd()

  return (
    <details
      className="chat-tool-summary"
      open={isOpen}
      onToggle={(event) => {
        setIsOpen(event.currentTarget.open)
      }}
    >
      <summary>
        {failedSuffix === undefined ? (
          summary
        ) : (
          <>
            <span className="chat-tool-summary-text">{summaryText}</span>
            <span className="chat-tool-summary-failed">{failedSuffix}</span>
          </>
        )}
      </summary>
      {children}
      {operations.length > 0 ? (
        <ul className="chat-tool-operations">
          {operations.map((operation, index) => (
            <li
              key={`${String(index)}-${operation.tool}-${operation.target ?? ''}`}
            >
              <span className="chat-tool-name">{operation.tool}</span>
              <code className="chat-tool-target">{operation.target}</code>
            </li>
          ))}
        </ul>
      ) : null}
    </details>
  )
}
