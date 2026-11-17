import '#components/tool-summary.css'

import { type ReactNode, useState } from 'react'

import type { ToolGroupItem, ToolOperation } from '#types'

type ToolSummaryOperation = Omit<ToolOperation, 'target'> & {
  target?: string | null
}

const fileEditToolNames = new Set([
  'Edit',
  'Write',
  'MultiEdit',
  'NotebookEdit',
  'apply_patch',
  'write_file',
  'edit_file',
  'FileChange',
  'fileChange',
])

type ToolSummaryProps = {
  summary: string
  operations: readonly ToolSummaryOperation[]
  items?: readonly ToolGroupItem[]
  children?: ReactNode
  open?: boolean
  showOperationStatus?: boolean
}

type ToolOperationRowProps = {
  operation: ToolSummaryOperation
  failedSuffix: string | undefined
  showOperationStatus: boolean
}

function ToolOperationRow({
  operation,
  failedSuffix,
  showOperationStatus,
}: ToolOperationRowProps) {
  return (
    <li
      data-has-target={
        typeof operation.target === 'string' && operation.target.length > 0
          ? 'true'
          : 'false'
      }
      data-failed={operation.failed === true ? 'true' : 'false'}
      data-has-status={
        !showOperationStatus ||
        (operation.failed === undefined && failedSuffix !== undefined)
          ? 'false'
          : undefined
      }
    >
      <span className="chat-tool-name">{operation.tool}</span>
      {typeof operation.target === 'string' && operation.target.length > 0 ? (
        <code className="chat-tool-target">{operation.target}</code>
      ) : null}
      {showOperationStatus &&
      (operation.failed !== undefined || failedSuffix === undefined) ? (
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
  let fileEditCount = 0
  let fileEditTarget: string | undefined
  const toolCounts = new Map<string, number>()
  for (const item of items) {
    if (item.type === 'tool_use') {
      if (fileEditToolNames.has(item.tool)) {
        fileEditCount += 1
        fileEditTarget ??= item.target
      } else {
        toolCounts.set(item.tool, (toolCounts.get(item.tool) ?? 0) + 1)
      }
    }
  }
  const fileEditSummary =
    fileEditCount === 1 && fileEditTarget !== undefined && fileEditTarget !== ''
      ? `Edit ${getFileName(fileEditTarget)}`
      : fileEditCount > 0
        ? `Edit ×${String(fileEditCount)}`
        : undefined
  const summary = [
    ...(thoughtCount > 0 ? [`Thought ×${String(thoughtCount)}`] : []),
    ...(fileEditSummary === undefined ? [] : [fileEditSummary]),
    ...[...toolCounts].map(([tool, count]) => `${tool} ×${String(count)}`),
  ]
  return summary.join(' · ')
}

function getFileName(target: string): string {
  const fileName = target.split(/[\\/]/).at(-1)
  return fileName === undefined || fileName === '' ? target : fileName
}

export function ToolSummary({
  summary,
  operations,
  items,
  children,
  open = false,
  showOperationStatus = true,
}: ToolSummaryProps) {
  const [isOpen, setIsOpen] = useState(open)
  const failedSuffix = summary.match(/· \d+ failed$/)?.[0]
  const summaryText =
    failedSuffix === undefined
      ? summary
      : summary.slice(0, -failedSuffix.length).trimEnd()
  const hasGroupedItems = items !== undefined && items.length > 0
  const hasOperations = operations.length > 0 || hasGroupedItems
  const thoughtCount =
    items?.filter((item) => item.type === 'thinking').length ?? 0
  const toolCallCount =
    items?.filter((item) => item.type === 'tool_use').length ?? 0
  const expandedSummary = hasGroupedItems
    ? [
        ...(thoughtCount > 0
          ? [`${String(thoughtCount)} thought${thoughtCount === 1 ? '' : 's'}`]
          : []),
        ...(toolCallCount > 0
          ? [
              `${String(toolCallCount)} tool call${toolCallCount === 1 ? '' : 's'}`,
            ]
          : []),
      ].join(' · ')
    : `${String(operations.length)} tool call${operations.length === 1 ? '' : 's'}`

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
        {hasOperations && isOpen ? (
          <>
            <span className="chat-tool-summary-text">{expandedSummary}</span>
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
                    <span className="chat-tool-thought-length">
                      {item.text.length} chars
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
                showOperationStatus={showOperationStatus}
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
              showOperationStatus={showOperationStatus}
            />
          ))}
        </ul>
      ) : null}
    </details>
  )
}
