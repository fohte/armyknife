import '#components/syntax-code-block.css'

import { Fragment, type ReactNode, useEffect, useState } from 'react'
import type { TokensResult } from 'shiki/types'

type SyntaxCodeBlockProps = {
  code: string
  language: string
  className?: string
  children?: ReactNode
  links?: SyntaxCodeBlockLink[]
}

export type SyntaxCodeBlockLink = {
  start: number
  end: number
  render: (children: ReactNode) => ReactNode
}

type HighlightedCode = {
  code: string
  language: string
  result: TokensResult
}

type HighlightToken = {
  color: string | undefined
  content: string
  end: number
  start: number
}

function flattenTokens(result: TokensResult): HighlightToken[] {
  const tokens: HighlightToken[] = []
  let offset = 0

  result.tokens.forEach((line, lineIndex) => {
    line.forEach((token) => {
      if (token.content.length === 0) return
      tokens.push({
        color: token.color,
        content: token.content,
        start: offset,
        end: offset + token.content.length,
      })
      offset += token.content.length
    })
    if (lineIndex < result.tokens.length - 1) {
      tokens.push({
        color: undefined,
        content: '\n',
        start: offset,
        end: offset + 1,
      })
      offset += 1
    }
  })

  return tokens
}

function renderTokenRange(
  tokens: HighlightToken[],
  start: number,
  end: number,
): ReactNode[] {
  return tokens.flatMap((token) => {
    const overlapStart = Math.max(start, token.start)
    const overlapEnd = Math.min(end, token.end)
    if (overlapStart >= overlapEnd) return []

    return [
      <span
        key={`${String(token.start)}-${String(overlapStart)}-${String(overlapEnd)}`}
        style={token.color === undefined ? undefined : { color: token.color }}
      >
        {token.content.slice(
          overlapStart - token.start,
          overlapEnd - token.start,
        )}
      </span>,
    ]
  })
}

function renderHighlightedCode(
  code: string,
  result: TokensResult,
  links: SyntaxCodeBlockLink[],
): ReactNode | null {
  const tokens = flattenTokens(result)
  if (tokens.map((token) => token.content).join('') !== code) return null

  const rendered: ReactNode[] = []
  let offset = 0

  links.forEach((link, linkIndex) => {
    const start = Math.max(offset, link.start)
    const end = Math.min(code.length, link.end)
    if (start > offset)
      rendered.push(...renderTokenRange(tokens, offset, start))
    if (end > start) {
      rendered.push(
        <Fragment key={`link-${String(linkIndex)}`}>
          {link.render(renderTokenRange(tokens, start, end))}
        </Fragment>,
      )
      offset = end
    }
  })

  if (offset < code.length) {
    rendered.push(...renderTokenRange(tokens, offset, code.length))
  }
  return rendered
}

export function SyntaxCodeBlock({
  code,
  language,
  className,
  children,
  links = [],
}: SyntaxCodeBlockProps) {
  const [highlightedCode, setHighlightedCode] =
    useState<HighlightedCode | null>(null)
  const highlighted =
    highlightedCode?.code === code && highlightedCode.language === language
      ? highlightedCode.result
      : null
  const highlightedContent =
    highlighted === null
      ? null
      : renderHighlightedCode(code, highlighted, links)

  useEffect(() => {
    let active = true
    void import('./syntax-highlighter')
      .then(({ highlightCode }) => highlightCode(code, language))
      .then((result) => {
        if (active && result !== null) {
          setHighlightedCode({ code, language, result })
        }
      })
      .catch((error: unknown) => {
        if (active) {
          console.error(
            'Syntax highlighting failed; showing plain text.',
            error,
          )
        }
      })

    return () => {
      active = false
    }
  }, [code, language])

  return (
    <pre
      className={['syntax-code-block', className].filter(Boolean).join(' ')}
      data-language={language}
    >
      <code className={`language-${language}`}>
        {highlightedContent ?? children ?? code}
      </code>
    </pre>
  )
}
