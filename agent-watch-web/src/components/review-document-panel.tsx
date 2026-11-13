import '#components/review-document-panel.css'

import { MarkdownMessage } from '#components/markdown-message'
import type { JsonValue } from '#types'

type ReviewDocumentPanelProps = {
  frontmatter: JsonValue
  body: string
}

type JsonObject = { [key: string]: JsonValue }

function isJsonObject(value: JsonValue): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function FrontmatterFields({
  value,
  nested = false,
}: {
  value: JsonObject
  nested?: boolean
}) {
  return (
    <dl
      className={`review-document-fields${nested ? ' review-document-fields--nested' : ''}`}
    >
      {Object.entries(value).map(([name, fieldValue]) =>
        isJsonObject(fieldValue) ? (
          <div className="review-document-field-group" key={name}>
            <dt className="review-document-group-title">{name}</dt>
            <dd className="review-document-group-content">
              <FrontmatterFields value={fieldValue} nested />
            </dd>
          </div>
        ) : (
          <div className="review-document-field" key={name}>
            <dt className="review-document-field-key">{name}</dt>
            <dd className="review-document-field-value">
              <FrontmatterValue value={fieldValue} />
            </dd>
          </div>
        ),
      )}
    </dl>
  )
}

function FrontmatterValue({ value }: { value: JsonValue }) {
  if (value === null) {
    return <span className="review-document-null">null</span>
  }
  if (typeof value === 'boolean') {
    return (
      <span
        className="review-document-boolean"
        data-checked={value}
        aria-label={String(value)}
      >
        {value ? '✓' : '−'}
      </span>
    )
  }
  if (typeof value === 'string') {
    return <span className="review-document-string">{value}</span>
  }
  if (typeof value === 'number') {
    return <span className="review-document-number">{String(value)}</span>
  }
  if (Array.isArray(value)) {
    return (
      <ul className="review-document-array">
        {value.map((item, index) => (
          <li key={index}>
            <FrontmatterValue value={item} />
          </li>
        ))}
      </ul>
    )
  }
  return <FrontmatterFields value={value} nested />
}

function ReviewDocumentFrontmatter({ value }: { value: JsonValue }) {
  return (
    <section
      className="review-document-frontmatter"
      aria-label="Document metadata"
    >
      <h2>Frontmatter</h2>
      {isJsonObject(value) ? (
        <FrontmatterFields value={value} />
      ) : (
        <FrontmatterValue value={value} />
      )}
    </section>
  )
}

export function ReviewDocumentPanel({
  frontmatter,
  body,
}: ReviewDocumentPanelProps) {
  return (
    <article className="review-document-panel">
      <ReviewDocumentFrontmatter value={frontmatter} />
      <MarkdownMessage className="review-document-body">{body}</MarkdownMessage>
    </article>
  )
}
