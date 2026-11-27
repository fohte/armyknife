import '#components/markdown-message.css'

import {
  Children,
  createContext,
  isValidElement,
  memo,
  type ReactNode,
  useContext,
} from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'

import { BangCommandActions } from '#components/bang-command'
import type {
  CommandOutputRefresh,
  CommandOutputStart,
} from '#components/command-result-message'
import { ImageLightbox } from '#components/image-lightbox'
import {
  InlineMarkdownAnchor,
  MarkdownAnchor,
} from '#components/markdown-anchor'
import { useMarkdownCommandContext } from '#components/markdown-command-context'
import {
  MarkdownComponentContext,
  type MarkdownComponentContextValue,
} from '#components/markdown-component-context'
import { MarkdownDetails } from '#components/markdown-details'
import {
  displayMarkdownImagePath,
  getMappedImageSource,
  isUnmappedLocalImagePath,
} from '#components/markdown-image-source'
import type {
  MarkdownFile,
  MarkdownNode,
  MarkdownRoot,
} from '#components/markdown-message-types'
import { useMarkdownSessionLinks } from '#components/markdown-session-links'
import { remarkMarkdownDetails } from '#components/remark-markdown-details'
import { SessionLink } from '#components/session-link'
import {
  SyntaxCodeBlock,
  type SyntaxCodeBlockLink,
} from '#components/syntax-code-block'
import { unwrapTaggedMessage } from '#components/unwrap-tagged-message'
import type { SessionLinkTarget } from '#types'

export { MarkdownSessionLinksProvider } from '#components/markdown-session-links'

const CodeBlockContext = createContext(false)

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
  imageSources?: Readonly<Record<string, string | null>>
  inline?: boolean
  mobile?: boolean
  sessionId?: string
  onCommandExecuted?: CommandOutputRefresh
  onCommandStarted?: CommandOutputStart
}

function textContent(value: ReactNode): string | undefined {
  if (typeof value === 'string') return value
  const parts = Children.toArray(value).map((child) => {
    if (typeof child === 'string' || typeof child === 'number')
      return String(child)
    if (isValidElement<{ children?: ReactNode }>(child)) {
      return textContent(child.props.children) ?? ''
    }
    return ''
  })
  return parts.length === 0 ? undefined : parts.join('')
}

function inlineBangCommand(code: string): string | null {
  const command = code.startsWith('! ') ? code.slice(2).trim() : ''
  return command.length === 0 ? null : command
}

function blockBangCommands(code: string): string[] {
  return code.split(/\r?\n/).flatMap((line) => {
    const command = line.startsWith('! ') ? line.slice(2).trim() : ''
    return command.length === 0 ? [] : [command]
  })
}

const MarkdownCode: NonNullable<Components['code']> = ({ node, ...props }) => {
  const isCodeBlock = useContext(CodeBlockContext)
  const { sessionsById, onSelectSession, commandContext, mobile } = useContext(
    MarkdownComponentContext,
  )
  if (node?.tagName !== 'code') return props.children
  const { className, children, ...codeProps } = props
  const text = typeof children === 'string' ? children : null
  const language = className?.match(/(?:^|\s)language-([\w-]+)/)?.[1]
  const parts = text === null ? [] : splitSessionIds(text, sessionsById)
  const codeChildren =
    parts.length === 0
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
        )
  let offset = 0
  const links = parts.flatMap((part, index): SyntaxCodeBlockLink[] => {
    const start = offset
    offset += part.text.length
    if (part.session === null) return []
    const session = part.session
    return [
      {
        start,
        end: offset,
        render: (linkChildren) => (
          <SessionLink
            key={String(index)}
            onSelectSession={onSelectSession}
            session={session}
          >
            {linkChildren}
          </SessionLink>
        ),
      },
    ]
  })
  const code =
    isCodeBlock && text !== null ? (
      <SyntaxCodeBlock
        code={text}
        language={language ?? 'text'}
        links={links}
        copyable
      >
        <>{codeChildren}</>
      </SyntaxCodeBlock>
    ) : (
      <code className={className} {...codeProps}>
        {codeChildren}
      </code>
    )
  if (commandContext === undefined || text === null || isCodeBlock) return code
  const command = inlineBangCommand(text)
  if (command === null) return code
  return (
    <span className="bang-command-inline">
      {code}
      <BangCommandActions
        commands={[command]}
        messageText={commandContext.messageText}
        sessionTitle={commandContext.sessionTitle}
        {...(mobile === undefined ? {} : { mobile })}
        {...(commandContext.onCommandExecuted === undefined
          ? {}
          : { onCommandExecuted: commandContext.onCommandExecuted })}
        {...(commandContext.onCommandStarted === undefined
          ? {}
          : { onCommandStarted: commandContext.onCommandStarted })}
        sessionId={commandContext.sessionId}
      />
    </span>
  )
}

const MarkdownPre: NonNullable<Components['pre']> = ({ children }) => {
  const { commandContext, mobile } = useContext(MarkdownComponentContext)
  const codeText = Children.toArray(children).flatMap((child) => {
    if (!isValidElement<{ children?: ReactNode }>(child)) return []
    return [textContent(child.props.children)]
  })[0]
  const commands = codeText === undefined ? [] : blockBangCommands(codeText)
  const codeBlock = (
    <CodeBlockContext.Provider value>{children}</CodeBlockContext.Provider>
  )
  if (commandContext === undefined || commands.length === 0) return codeBlock
  return (
    <div className="markdown-code-block">
      {codeBlock}
      <BangCommandActions
        commands={commands}
        messageText={commandContext.messageText}
        sessionTitle={commandContext.sessionTitle}
        {...(mobile === undefined ? {} : { mobile })}
        {...(commandContext.onCommandExecuted === undefined
          ? {}
          : { onCommandExecuted: commandContext.onCommandExecuted })}
        {...(commandContext.onCommandStarted === undefined
          ? {}
          : { onCommandStarted: commandContext.onCommandStarted })}
        sessionId={commandContext.sessionId}
      />
    </div>
  )
}

const MarkdownParagraph: NonNullable<Components['p']> = ({ children }) => (
  <span>{children}</span>
)

const MarkdownImage: NonNullable<Components['img']> = ({ src, alt, title }) => {
  const { imageSources } = useContext(MarkdownComponentContext)
  if (src === undefined || src === '') return null
  const mappedImage = getMappedImageSource(src, imageSources)
  if (
    mappedImage?.source === null ||
    (mappedImage === undefined &&
      imageSources !== undefined &&
      isUnmappedLocalImagePath(src))
  ) {
    return (
      <span className="markdown-image-unavailable">
        表示できません: {mappedImage?.path ?? displayMarkdownImagePath(src)}
      </span>
    )
  }

  return (
    <ImageLightbox
      src={mappedImage?.source ?? src}
      alt={alt ?? ''}
      {...(title === undefined ? {} : { title })}
      loading="lazy"
      decoding="async"
      imageClassName="chat-user-image"
    />
  )
}

const markdownComponents: Components = {
  a: MarkdownAnchor,
  code: MarkdownCode,
  details: MarkdownDetails,
  img: MarkdownImage,
  pre: MarkdownPre,
}

const inlineMarkdownComponents: Components = {
  ...markdownComponents,
  p: MarkdownParagraph,
  a: InlineMarkdownAnchor,
}

export const MarkdownMessage = memo(function MarkdownMessage({
  children,
  className,
  imageSources,
  inline = false,
  mobile = false,
  sessionId,
  onCommandExecuted,
  onCommandStarted,
}: MarkdownMessageProps) {
  const sessionLinks = useMarkdownSessionLinks()
  const inheritedCommandContext = useMarkdownCommandContext()
  const commandSessionId = sessionId ?? inheritedCommandContext?.sessionId
  const commandSessionTitle = inheritedCommandContext?.sessionTitle ?? 'Session'
  const commandExecutedCallback =
    onCommandExecuted ?? inheritedCommandContext?.onCommandExecuted
  const commandStartedCallback =
    onCommandStarted ?? inheritedCommandContext?.onCommandStarted
  const rootClassName =
    className === undefined
      ? 'markdown-message'
      : `markdown-message ${className}`

  const sessionsById = new Map(
    sessionLinks.sessions.map((session) => [session.id.toLowerCase(), session]),
  )
  const markdownSource = inline ? children : unwrapTaggedMessage(children)
  const componentContext: MarkdownComponentContextValue = {
    sessionsById,
    webUis: sessionLinks.webUis ?? [],
    ...(imageSources === undefined ? {} : { imageSources }),
    mobile,
    ...(sessionLinks.onSelectSession === undefined
      ? {}
      : { onSelectSession: sessionLinks.onSelectSession }),
    ...(sessionLinks.onOpenWebUi === undefined
      ? {}
      : { onOpenWebUi: sessionLinks.onOpenWebUi }),
    ...(commandSessionId === undefined
      ? {}
      : {
          commandContext: {
            messageText: children,
            sessionId: commandSessionId,
            sessionTitle: commandSessionTitle,
            ...(commandExecutedCallback === undefined
              ? {}
              : { onCommandExecuted: commandExecutedCallback }),
            ...(commandStartedCallback === undefined
              ? {}
              : { onCommandStarted: commandStartedCallback }),
          },
        }),
  }
  const message = (
    <MarkdownComponentContext.Provider value={componentContext}>
      <ReactMarkdown
        components={inline ? inlineMarkdownComponents : markdownComponents}
        remarkPlugins={[
          remarkGfm,
          remarkMarkdownDetails,
          remarkHtmlBreaks,
          () => remarkLinkSessionIds(sessionsById),
          remarkTrimNonAsciiAutolinks,
        ]}
      >
        {markdownSource}
      </ReactMarkdown>
    </MarkdownComponentContext.Provider>
  )

  return inline ? (
    <span className={rootClassName}>{message}</span>
  ) : (
    <div className={rootClassName}>{message}</div>
  )
})
