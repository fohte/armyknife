import '#components/tool-summary.css'

import hljs from 'highlight.js/lib/core'
import bash from 'highlight.js/lib/languages/bash'
import { useState } from 'react'

import type { ToolOperation } from '#types'

const shellHighlighter = hljs.newInstance()
shellHighlighter.registerLanguage('bash', bash)

function isShellTool(tool: string) {
  return ['Bash', 'bash', 'exec', 'shell', 'CommandExecution'].includes(tool)
}

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
  const [isOpen, setIsOpen] = useState(open)

  return (
    <details
      className="chat-tool-summary"
      open={isOpen}
      onToggle={(event) => {
        setIsOpen(event.currentTarget.open)
      }}
    >
      <summary>{summary}</summary>
      {operations.length > 0 ? (
        <ul className="chat-tool-operations">
          {operations.map((operation, index) => (
            <li
              key={`${String(index)}-${operation.tool}-${operation.target ?? ''}`}
            >
              <span className="chat-tool-name">{operation.tool}</span>
              {operation.target != null && operation.target !== '' ? (
                isShellTool(operation.tool) && isOpen ? (
                  <code
                    className="chat-tool-command hljs"
                    dangerouslySetInnerHTML={{
                      __html: shellHighlighter.highlight(operation.target, {
                        language: 'bash',
                      }).value,
                    }}
                  />
                ) : (
                  <code>{operation.target}</code>
                )
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}
    </details>
  )
}
