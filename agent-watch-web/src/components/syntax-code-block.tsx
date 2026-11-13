import '#components/syntax-code-block.css'

import { type ReactNode, useEffect, useState } from 'react'
import type { TokensResult } from 'shiki/types'

type SyntaxCodeBlockProps = {
  code: string
  language: string
  className?: string
  children?: ReactNode
}

type HighlightedCode = {
  code: string
  language: string
  result: TokensResult
}

export function SyntaxCodeBlock({
  code,
  language,
  className,
  children,
}: SyntaxCodeBlockProps) {
  const [highlightedCode, setHighlightedCode] =
    useState<HighlightedCode | null>(null)
  const highlighted =
    highlightedCode?.code === code && highlightedCode.language === language
      ? highlightedCode.result
      : null

  useEffect(() => {
    let active = true
    void import('./syntax-highlighter')
      .then(({ highlightCode }) => highlightCode(code, language))
      .then((result) => {
        if (active && result !== null) {
          setHighlightedCode({ code, language, result })
        }
      })
      .catch(() => undefined)

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
        {highlighted === null
          ? (children ?? code)
          : highlighted.tokens.map((line, lineIndex) => (
              <span key={String(lineIndex)}>
                {line.map((token, tokenIndex) => (
                  <span
                    key={`${String(lineIndex)}-${String(tokenIndex)}`}
                    style={
                      token.color === undefined
                        ? undefined
                        : { color: token.color }
                    }
                  >
                    {token.content}
                  </span>
                ))}
                {lineIndex < highlighted.tokens.length - 1 ? '\n' : null}
              </span>
            ))}
      </code>
    </pre>
  )
}
