import '#components/tool-summary.css'

import { type ReactNode, useState } from 'react'

import type { ToolOperation } from '#types'

type ToolSummaryOperation = Omit<ToolOperation, 'target'> & {
  target?: string | null
}

type ToolSummaryProps = {
  summary: string
  operations: readonly ToolSummaryOperation[]
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
      data-has-operations={operations.length > 0 ? 'true' : 'false'}
      open={isOpen}
      onToggle={(event) => {
        setIsOpen(event.currentTarget.open)
      }}
    >
      <summary>
        {operations.length > 0 && isOpen ? (
          <>
            <span>
              {operations.length} tool call{operations.length === 1 ? '' : 's'}
            </span>
            <span className="chat-tool-summary-collapse">‹ Collapse</span>
          </>
        ) : failedSuffix === undefined ? (
          operations.length > 0 ? (
            <span className="chat-tool-summary-text">{summary}</span>
          ) : (
            summary
          )
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
              data-has-target={
                typeof operation.target === 'string' &&
                operation.target.length > 0
                  ? 'true'
                  : 'false'
              }
              data-failed={operation.failed === true ? 'true' : 'false'}
            >
              <span className="chat-tool-name">{operation.tool}</span>
              {typeof operation.target === 'string' &&
              operation.target.length > 0 ? (
                <code className="chat-tool-target">{operation.target}</code>
              ) : null}
              <span
                className="chat-tool-operation-status"
                role="img"
                aria-label={operation.failed === true ? 'Failed' : 'Succeeded'}
              >
                {operation.failed === true ? '✕' : '✓'}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
    </details>
  )
}
