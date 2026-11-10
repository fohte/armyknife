import '#components/markdown-message.css'

import { memo } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'

type MarkdownNode = {
  children?: MarkdownNode[]
  position?: {
    start: { offset?: number }
    end: { offset?: number }
  }
  type: string
  url?: string
  value?: string
}

type MarkdownRoot = MarkdownNode & {
  children: MarkdownNode[]
}

type MarkdownFile = {
  value: string | Uint8Array
}

function remarkTrimNonAsciiAutolinks() {
  return (tree: MarkdownRoot, file: MarkdownFile) => {
    const source =
      typeof file.value === 'string'
        ? file.value
        : new TextDecoder().decode(file.value)

    function visit(node: MarkdownNode) {
      const children = node.children
      if (children === undefined) return

      for (let index = 0; index < children.length; index += 1) {
        const child = children[index]
        if (child === undefined) continue

        const text = child.children?.[0]
        const start = child.position?.start.offset
        const end = child.position?.end.offset
        const raw =
          start === undefined || end === undefined
            ? undefined
            : source.slice(start, end)

        if (
          child.type === 'link' &&
          child.url !== undefined &&
          child.children?.length === 1 &&
          text?.type === 'text' &&
          text.value === child.url &&
          raw === child.url
        ) {
          // Source equality excludes explicit Markdown links, whose source includes link syntax.
          const splitAt = child.url.search(/[^\x00-\x7F]/)
          if (splitAt !== -1) {
            const suffix = child.url.slice(splitAt)
            child.url = child.url.slice(0, splitAt)
            text.value = child.url
            children.splice(index + 1, 0, { type: 'text', value: suffix })
            index += 1
          }
        }

        visit(child)
      }
    }

    visit(tree)
  }
}

type MarkdownMessageProps = {
  children: string
  className?: string
  inline?: boolean
}

const markdownComponents: Components = {
  a: ({ node, ...props }) =>
    node?.tagName === 'a' ? (
      <a {...props} target="_blank" rel="noopener noreferrer" />
    ) : null,
}

const inlineMarkdownComponents: Components = {
  ...markdownComponents,
  p: ({ children }) => <span>{children}</span>,
  a: ({ node, href, children }) =>
    node?.tagName === 'a' ? (
      <span className="markdown-link-text">{href ?? children}</span>
    ) : null,
}

export const MarkdownMessage = memo(function MarkdownMessage({
  children,
  className,
  inline = false,
}: MarkdownMessageProps) {
  const rootClassName =
    className === undefined
      ? 'markdown-message'
      : `markdown-message ${className}`

  const message = (
    <ReactMarkdown
      components={inline ? inlineMarkdownComponents : markdownComponents}
      remarkPlugins={[remarkGfm, remarkTrimNonAsciiAutolinks]}
    >
      {children}
    </ReactMarkdown>
  )

  return inline ? (
    <span className={rootClassName}>{message}</span>
  ) : (
    <div className={rootClassName}>{message}</div>
  )
})
