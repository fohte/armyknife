import '#components/collapsible-text.css'

import { useId, useLayoutEffect, useRef, useState } from 'react'

type CollapsibleTextProps = {
  text: string
  expanded: boolean
  collapsedLines?: number
  className?: string
  onExpandedChange: (expanded: boolean) => void
}

export function CollapsibleText({
  text,
  expanded,
  collapsedLines = 2,
  className,
  onExpandedChange,
}: CollapsibleTextProps) {
  const contentId = useId()
  const contentRef = useRef<HTMLParagraphElement>(null)
  const [overflows, setOverflows] = useState(false)

  useLayoutEffect(() => {
    const content = contentRef.current
    if (content === null) return
    const fonts = document.fonts
    let isActive = true

    content.style.setProperty(
      '--collapsible-text-lines',
      String(collapsedLines),
    )

    const measure = () => {
      if (!isActive) return
      const lineHeight = Number.parseFloat(getComputedStyle(content).lineHeight)
      if (!Number.isFinite(lineHeight)) return

      setOverflows(content.scrollHeight > lineHeight * collapsedLines + 1)
    }

    measure()
    fonts.addEventListener('loadingdone', measure)
    const resizeObserver = new ResizeObserver(() => {
      measure()
    })
    resizeObserver.observe(content)
    return () => {
      isActive = false
      fonts.removeEventListener('loadingdone', measure)
      resizeObserver.disconnect()
    }
  }, [collapsedLines, expanded, text])

  return (
    <div
      className={`collapsible-text${className === undefined ? '' : ` ${className}`}`}
    >
      <p
        ref={contentRef}
        id={contentId}
        className="collapsible-text__content"
        data-expanded={expanded}
      >
        {text}
      </p>
      {overflows ? (
        <button
          className="collapsible-text__toggle"
          type="button"
          aria-controls={contentId}
          aria-expanded={expanded}
          onClick={() => {
            onExpandedChange(!expanded)
          }}
        >
          {expanded ? 'Show less' : 'Show more'}
        </button>
      ) : null}
    </div>
  )
}
