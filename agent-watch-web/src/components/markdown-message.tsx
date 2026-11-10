import '#components/markdown-message.css'

import { memo } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'

type MarkdownMessageProps = {
  children: string
  className?: string
}

const markdownComponents: Components = {
  a: ({ node, ...props }) =>
    node?.tagName === 'a' ? (
      <a {...props} target="_blank" rel="noopener noreferrer" />
    ) : null,
}

export const MarkdownMessage = memo(function MarkdownMessage({
  children,
  className,
}: MarkdownMessageProps) {
  const rootClassName =
    className === undefined
      ? 'markdown-message'
      : `markdown-message ${className}`

  return (
    <div className={rootClassName}>
      <ReactMarkdown
        components={markdownComponents}
        remarkPlugins={[remarkGfm]}
      >
        {children}
      </ReactMarkdown>
    </div>
  )
})
