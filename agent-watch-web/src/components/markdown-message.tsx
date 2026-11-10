import '#components/markdown-message.css'

import { memo } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

type MarkdownMessageProps = {
  children: string
  className?: string
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
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{children}</ReactMarkdown>
    </div>
  )
})
