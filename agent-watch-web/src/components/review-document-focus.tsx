import '#components/review-document-focus.css'

import { ChevronLeft } from 'lucide-react'
import { useEffect } from 'react'

import { MarkdownMessage } from '#components/markdown-message'

type ReviewDocumentFocusProps = {
  sessionTitle: string
  body: string
  onBack: () => void
}

export function ReviewDocumentFocus({
  sessionTitle,
  body,
  onBack,
}: ReviewDocumentFocusProps) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      onBack()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [onBack])

  return (
    <section
      aria-label="Draft body, full width"
      className="review-document-focus"
    >
      <header className="review-document-focus-header">
        <button
          aria-label="Back to conversation"
          className="review-document-focus-back"
          type="button"
          onClick={onBack}
        >
          <ChevronLeft aria-hidden="true" size={18} />
          Conversation
        </button>
        <span className="review-document-focus-session">{sessionTitle}</span>
        <span aria-hidden="true" className="review-document-focus-divider">
          ·
        </span>
        <strong>Draft</strong>
        <kbd className="review-document-focus-escape">
          <kbd>Esc</kbd> Back
        </kbd>
      </header>
      <div className="review-document-focus-columns">
        <section
          aria-label="Markdown"
          className="review-document-focus-column review-document-focus-markdown"
        >
          <h2>MARKDOWN</h2>
          <pre aria-label="Markdown body">{body}</pre>
        </section>
        <section
          aria-label="Preview"
          className="review-document-focus-column review-document-focus-preview"
        >
          <h2>PREVIEW</h2>
          <div className="review-document-focus-rendered">
            <MarkdownMessage className="review-document-body">
              {body}
            </MarkdownMessage>
          </div>
        </section>
      </div>
    </section>
  )
}
