import '#components/markdown-message.css'

import { memo, type ReactNode } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'

import { SessionLink } from '#components/session-link'
import type { SessionLinkTarget } from '#types'

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

type SessionLinkTextPart = {
  session: SessionLinkTarget | null
  text: string
}

function remarkHtmlBreaks() {
  return (tree: MarkdownRoot) => {
    function visit(node: MarkdownNode) {
      if (
        node.type === 'html' &&
        /^<br\s*\/?>$/i.test(node.value?.trim() ?? '')
      ) {
        node.type = 'break'
        delete node.value
        return
      }

      node.children?.forEach(visit)
    }

    visit(tree)
  }
}

const sessionIdPattern =
  /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi

function splitSessionIds(
  value: string,
  sessionsById: ReadonlyMap<string, SessionLinkTarget>,
): SessionLinkTextPart[] {
  const parts: SessionLinkTextPart[] = []
  let previousEnd = 0

  for (const match of value.matchAll(sessionIdPattern)) {
    const matchedId = match[0]
    const start = match.index
    const end = start + matchedId.length
    const hasHexBefore = /[0-9a-f]/i.test(value[start - 1] ?? '')
    const hasHexAfter = /[0-9a-f]/i.test(value[end] ?? '')
    const session = sessionsById.get(matchedId.toLowerCase())

    if (hasHexBefore || hasHexAfter || session === undefined) continue

    if (start > previousEnd) {
      parts.push({ session: null, text: value.slice(previousEnd, start) })
    }
    parts.push({ session, text: matchedId })
    previousEnd = end
  }

  if (previousEnd < value.length) {
    parts.push({ session: null, text: value.slice(previousEnd) })
  }

  return parts.length > 0 ? parts : [{ session: null, text: value }]
}

function remarkLinkSessionIds(
  sessionsById: ReadonlyMap<string, SessionLinkTarget>,
) {
  return (tree: MarkdownRoot) => {
    function visit(node: MarkdownNode) {
      const children = node.children
      if (
        children === undefined ||
        node.type === 'link' ||
        node.type === 'linkReference'
      ) {
        return
      }

      const nextChildren: MarkdownNode[] = []
      for (const child of children) {
        if (child.type !== 'text' || child.value === undefined) {
          visit(child)
          nextChildren.push(child)
          continue
        }

        const parts = splitSessionIds(child.value, sessionsById)
        if (!parts.some((part) => part.session !== null)) {
          nextChildren.push(child)
          continue
        }

        for (const part of parts) {
          nextChildren.push(
            part.session === null
              ? { type: 'text', value: part.text }
              : {
                  type: 'link',
                  url: `https://watch-session.invalid/${part.session.id.toLowerCase()}`,
                  children: [{ type: 'text', value: part.text }],
                },
          )
        }
      }
      node.children = nextChildren
    }

    visit(tree)
  }
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
  onSelectSession?: (id: string) => void
  sessions?: readonly SessionLinkTarget[]
}

function createMarkdownComponents(
  sessionsById: ReadonlyMap<string, SessionLinkTarget>,
  onSelectSession?: (id: string) => void,
): { markdown: Components; inline: Components } {
  const renderLink = (href: string | undefined, children: ReactNode) => {
    const sessionId = href?.match(
      /^https:\/\/watch-session\.invalid\/([0-9a-f-]+)$/i,
    )?.[1]
    if (sessionId !== undefined) {
      const session = sessionsById.get(sessionId.toLowerCase())
      return session === undefined ? (
        children
      ) : (
        <SessionLink onSelectSession={onSelectSession} session={session}>
          {children}
        </SessionLink>
      )
    }
    return null
  }

  const renderCode: NonNullable<Components['code']> = ({ node, ...props }) => {
    if (node?.tagName !== 'code') return props.children
    const { className, children, ...codeProps } = props
    const text = typeof children === 'string' ? children : null
    const parts = text === null ? [] : splitSessionIds(text, sessionsById)
    return (
      <code className={className} {...codeProps}>
        {parts.length === 0
          ? children
          : parts.map((part, index) =>
              part.session === null ? (
                part.text
              ) : (
                <SessionLink
                  key={String(index)}
                  onSelectSession={onSelectSession}
                  session={part.session}
                >
                  {part.text}
                </SessionLink>
              ),
            )}
      </code>
    )
  }

  const markdown: Components = {
    a: ({ node, href, children, ...props }) => {
      if (node?.tagName !== 'a') return null
      const sessionLink = renderLink(href, children)
      return (
        sessionLink ?? (
          <a {...props} href={href} target="_blank" rel="noopener noreferrer">
            {children}
          </a>
        )
      )
    },
    code: renderCode,
  }

  return {
    markdown,
    inline: {
      ...markdown,
      p: ({ children }) => <span>{children}</span>,
      a: ({ node, href, children }) => {
        if (node?.tagName !== 'a') return null
        return (
          renderLink(href, children) ?? (
            <span className="markdown-link-text">{href ?? children}</span>
          )
        )
      },
    },
  }
}

export const MarkdownMessage = memo(function MarkdownMessage({
  children,
  className,
  inline = false,
  onSelectSession,
  sessions = [],
}: MarkdownMessageProps) {
  const rootClassName =
    className === undefined
      ? 'markdown-message'
      : `markdown-message ${className}`

  const sessionsById = new Map(
    sessions.map((session) => [session.id.toLowerCase(), session]),
  )
  const components = createMarkdownComponents(sessionsById, onSelectSession)
  const message = (
    <ReactMarkdown
      components={inline ? components.inline : components.markdown}
      remarkPlugins={[
        remarkGfm,
        remarkHtmlBreaks,
        () => remarkLinkSessionIds(sessionsById),
        remarkTrimNonAsciiAutolinks,
      ]}
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
