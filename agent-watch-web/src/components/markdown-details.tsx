import { Children, type ReactNode, useState } from 'react'

type MarkdownDetailsProps = {
  children?: ReactNode
  open?: boolean | undefined
}

export function MarkdownDetails({
  children,
  open = false,
}: MarkdownDetailsProps) {
  const [expanded, setExpanded] = useState(open)
  const [summary, ...body] = Children.toArray(children)

  return (
    <div className="markdown-details">
      <button
        aria-expanded={expanded}
        className="markdown-details-summary"
        onClick={() => {
          setExpanded((current) => !current)
        }}
        type="button"
      >
        <svg
          aria-hidden="true"
          className="markdown-details-chevron"
          data-expanded={expanded ? 'true' : undefined}
          viewBox="0 0 10 10"
        >
          <path d="M2 1l6 4-6 4z" fill="currentColor" />
        </svg>
        {summary}
      </button>
      {expanded ? <div className="markdown-details-body">{body}</div> : null}
    </div>
  )
}
