import { type MouseEvent, type ReactNode, useContext } from 'react'
import type { Components } from 'react-markdown'

import { MarkdownComponentContext } from '#components/markdown-component-context'
import { SessionLink } from '#components/session-link'
import { findWebUiLink } from '#components/web-ui-link-target'
import type { SessionLinkTarget } from '#types'

type MarkdownImageNode = {
  children?: MarkdownImageNode[]
  tagName?: string
  type: string
}

function renderSessionLink(
  href: string | undefined,
  children: ReactNode,
  sessionsById: ReadonlyMap<string, SessionLinkTarget>,
  onSelectSession?: (id: string) => void,
) {
  const sessionId = href?.match(
    /^https:\/\/watch-session\.invalid\/([0-9a-f-]+)$/i,
  )?.[1]
  if (sessionId === undefined) return null
  const session = sessionsById.get(sessionId.toLowerCase())
  return session === undefined ? (
    children
  ) : (
    <SessionLink onSelectSession={onSelectSession} session={session}>
      {children}
    </SessionLink>
  )
}

function containsMarkdownImage(node: MarkdownImageNode): boolean {
  return (
    node.tagName === 'img' ||
    (node.children?.some(containsMarkdownImage) ?? false)
  )
}

export const MarkdownAnchor: NonNullable<Components['a']> = ({
  node,
  href,
  children,
  ...props
}) => {
  const { sessionsById, onSelectSession, webUis, onOpenWebUi } = useContext(
    MarkdownComponentContext,
  )
  if (node?.tagName !== 'a') return null
  const sessionLink = renderSessionLink(
    href,
    children,
    sessionsById,
    onSelectSession,
  )
  const webUiLink = findWebUiLink(href, webUis)
  if (sessionLink === null && webUiLink !== null) {
    const handleClick = (event: MouseEvent<HTMLAnchorElement>) => {
      if (onOpenWebUi === undefined || href === undefined) return
      event.preventDefault()
      onOpenWebUi(webUiLink.webUi, href)
    }
    return (
      <span className="markdown-web-ui-link">
        <a
          {...props}
          href={href}
          {...(onOpenWebUi === undefined
            ? { target: '_blank', rel: 'noopener noreferrer' }
            : { onClick: handleClick })}
        >
          {children}
        </a>
        <span className="markdown-web-ui-link-name">
          {webUiLink.webUi.name}
        </span>
      </span>
    )
  }
  if (
    sessionLink === null &&
    href !== '' &&
    node.children.some(containsMarkdownImage)
  ) {
    return (
      <span className="markdown-linked-image">
        {children}
        <a
          className="markdown-image-link"
          href={href}
          target="_blank"
          rel="noopener noreferrer"
        >
          Open link
        </a>
      </span>
    )
  }
  return (
    sessionLink ?? (
      <a {...props} href={href} target="_blank" rel="noopener noreferrer">
        {children}
      </a>
    )
  )
}

export const InlineMarkdownAnchor: NonNullable<Components['a']> = ({
  node,
  href,
  children,
}) => {
  const { sessionsById, onSelectSession } = useContext(MarkdownComponentContext)
  if (node?.tagName !== 'a') return null
  return (
    renderSessionLink(href, children, sessionsById, onSelectSession) ?? (
      <span className="markdown-link-text">{href ?? children}</span>
    )
  )
}
