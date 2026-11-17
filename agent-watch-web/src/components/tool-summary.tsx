import '#components/tool-summary.css'

import { type ReactNode, useState } from 'react'

import type { ToolGroupItem, ToolOperation } from '#types'

type ToolSummaryOperation = Omit<ToolOperation, 'target'> & {
  target?: string | null
}

type ToolSummaryProps = {
  summary: string
  operations: readonly ToolSummaryOperation[]
  items?: readonly ToolGroupItem[]
  children?: ReactNode
  open?: boolean
}

type ToolOperationRowProps = {
  operation: ToolSummaryOperation
  failedSuffix: string | undefined
}

function ToolOperationRow({ operation, failedSuffix }: ToolOperationRowProps) {
  return (
    <li
      data-has-target={
        typeof operation.target === 'string' && operation.target.length > 0
          ? 'true'
          : 'false'
      }
      data-failed={operation.failed === true ? 'true' : 'false'}
      data-has-status={
        operation.failed === undefined && failedSuffix !== undefined
          ? 'false'
          : undefined
      }
    >
      <span className="chat-tool-name">{operation.tool}</span>
      {typeof operation.target === 'string' && operation.target.length > 0 ? (
        <code className="chat-tool-target">{operation.target}</code>
      ) : null}
      {operation.failed !== undefined || failedSuffix === undefined ? (
        <span
          className="chat-tool-operation-status"
          role="img"
          aria-label={operation.failed === true ? 'Failed' : 'Succeeded'}
        >
          {operation.failed === true ? '✕' : '✓'}
        </span>
      ) : null}
    </li>
  )
}

export function summarizeToolGroupItems(
  items: readonly ToolGroupItem[],
): string {
  const thoughtCount = items.filter((item) => item.type === 'thinking').length
  const tools = new Map<string, { count: number; target: string | undefined }>()
  for (const item of items) {
    if (item.type === 'tool_use') {
      const existing = tools.get(item.tool)
      tools.set(item.tool, {
        count: (existing?.count ?? 0) + 1,
        target: existing?.target ?? item.target,
      })
    }
  }
  const summary = [
    ...(thoughtCount > 0 ? [`Thought ×${String(thoughtCount)}`] : []),
    ...[...tools].map(([tool, { count, target }]) => {
      if (
        tool === 'Edit' &&
        count === 1 &&
        target !== undefined &&
        target !== ''
      ) {
        const fileName = target.split(/[\\/]/).at(-1)
        return `Edit ${fileName === undefined || fileName === '' ? target : fileName}`
      }
      return `${tool} ×${String(count)}`
    }),
  ]
  return summary.join(' · ')
}

export function ToolSummary({
  summary,
  operations,
  items,
  children,
  open = false,
}: ToolSummaryProps) {
  const [isOpen, setIsOpen] = useState(open)
  const failedSuffix = summary.match(/· \d+ failed$/)?.[0]
  const summaryText =
    failedSuffix === undefined
      ? summary
      : summary.slice(0, -failedSuffix.length).trimEnd()
  const hasGroupedItems = items !== undefined && items.length > 0
  const hasOperations = operations.length > 0 || hasGroupedItems

  return (
    <details
      className="chat-tool-summary"
      data-has-operations={hasOperations ? 'true' : 'false'}
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
        ) : hasGroupedItems && isOpen ? (
          <>
            <span className="chat-tool-summary-text">{summaryText}</span>
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
      {hasGroupedItems ? (
        <ul className="chat-tool-operations">
          {items.map((item, index) =>
            item.type === 'thinking' ? (
              <li
                className="chat-tool-thought"
                key={`thought-${String(index)}`}
              >
                <details>
                  <summary>
                    <span className="chat-tool-name">Thought</span>
                    <span className="chat-tool-thought-preview">
                      {item.text}
                    </span>
                  </summary>
                  <p className="chat-tool-thought-full">{item.text}</p>
                </details>
              </li>
            ) : (
              <ToolOperationRow
                key={`${String(index)}-${item.tool}-${item.target ?? ''}`}
                operation={item}
                failedSuffix={undefined}
              />
            ),
          )}
        </ul>
      ) : operations.length > 0 ? (
        <ul className="chat-tool-operations">
          {operations.map((operation, index) => (
            <ToolOperationRow
              key={`${String(index)}-${operation.tool}-${operation.target ?? ''}`}
              operation={operation}
              failedSuffix={failedSuffix}
            />
          ))}
        </ul>
      ) : null}
    </details>
  )
}
