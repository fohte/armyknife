import '#components/tool-summary.css'

import type { ToolOperation } from '#types'

type ToolSummaryProps = {
  summary: string
  operations: readonly ToolOperation[]
  open?: boolean
}

export function ToolSummary({
  summary,
  operations,
  open = false,
}: ToolSummaryProps) {
  return (
    <details className="chat-tool-summary" open={open}>
      <summary>{summary}</summary>
      {operations.length > 0 ? (
        <ul className="chat-tool-operations">
          {operations.map((operation, index) => (
            <li
              key={`${String(index)}-${operation.tool}-${operation.target ?? ''}`}
            >
              <span className="chat-tool-name">{operation.tool}</span>
              {operation.target != null && operation.target !== '' ? (
                <code>{operation.target}</code>
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}
    </details>
  )
}
