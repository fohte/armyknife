import '#components/review-document-preview.css'
import '#components/review-document-review-thread-preview.css'

import { File } from 'lucide-react'
import { useLayoutEffect, useRef, useState } from 'react'

import { MarkdownMessage } from '#components/markdown-message'
import {
  parseIssueComment,
  parseReviewMessages,
  parseReviewThreads,
  updateReviewThread,
} from '#components/review-document-preview-data'
import type { ReviewDocumentSummary } from '#components/review-document-summary'
import type { JsonValue } from '#types'

type ReviewDocumentPreviewProps = {
  summary?: ReviewDocumentSummary
  body: string
  frontmatter?: JsonValue
  mode?: 'body' | 'frontmatter'
  disabled?: boolean
  onFrontmatterChange?: (frontmatter: JsonValue) => void
  canSave?: boolean
  onBodyChange?: (body: string) => void
}

type JsonObject = { [key: string]: JsonValue }

function isJsonObject(value: JsonValue): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function stringList(value: JsonValue | undefined): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === 'string')
    : []
}

function issueReferenceUrl(reference: string) {
  const match = /^([^/]+)\/([^/#]+)#(\d+)$/.exec(reference)
  const owner = match?.[1]
  const repository = match?.[2]
  const issueNumber = match?.[3]
  if (
    owner === undefined ||
    repository === undefined ||
    issueNumber === undefined
  )
    return null
  return `https://github.com/${encodeURIComponent(owner)}/${encodeURIComponent(repository)}/issues/${issueNumber}`
}

function IssueReference({ reference }: { reference: string }) {
  const href = issueReferenceUrl(reference)
  return href === null ? (
    <code>{reference}</code>
  ) : (
    <a
      className="review-document-issue-reference-link"
      href={href}
      rel="noopener noreferrer"
      target="_blank"
    >
      {reference}
    </a>
  )
}

function AddIssueValue({
  buttonLabel,
  inputLabel,
  placeholder,
  parent = false,
  disabled,
  onAdd,
}: {
  buttonLabel: string
  inputLabel: string
  placeholder: string
  parent?: boolean
  disabled: boolean
  onAdd: (value: string) => void
}) {
  const [adding, setAdding] = useState(false)
  const [value, setValue] = useState('')
  const commit = () => {
    const nextValue = value.trim()
    if (nextValue.length > 0) onAdd(nextValue)
    setValue('')
    setAdding(false)
  }
  if (adding) {
    return (
      <input
        aria-label={inputLabel}
        autoFocus
        className={`review-document-issue-add-field${parent ? ' review-document-issue-add-field--parent' : ''}`}
        disabled={disabled}
        placeholder={placeholder}
        value={value}
        onChange={(event) => {
          setValue(event.currentTarget.value)
        }}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            setValue('')
            setAdding(false)
          }
          if (event.key !== 'Enter') return
          event.preventDefault()
          commit()
        }}
      />
    )
  }
  return (
    <button
      aria-label={inputLabel}
      className={`review-document-issue-add-button${parent ? ' review-document-issue-add-button--parent' : ''}`}
      disabled={disabled}
      type="button"
      onClick={() => {
        setAdding(true)
      }}
    >
      {buttonLabel}
    </button>
  )
}

function IssueFrontmatter({
  frontmatter,
  disabled,
  onChange,
}: {
  frontmatter: JsonValue
  disabled: boolean
  onChange: (frontmatter: JsonValue) => void
}) {
  const values = isJsonObject(frontmatter) ? frontmatter : {}
  const title = typeof values['title'] === 'string' ? values['title'] : ''
  const labels = stringList(values['labels'])
  const assignees = stringList(values['assignees'])
  const parentIssue =
    typeof values['parentIssue'] === 'string' ? values['parentIssue'] : ''
  const subIssues = stringList(values['subIssues'])
  const update = (name: string, value: JsonValue) => {
    onChange({ ...values, [name]: value })
  }
  const addListValue = (
    name: 'labels' | 'assignees' | 'subIssues',
    value: string,
  ) => {
    const nextValue = value.trim()
    if (nextValue.length > 0)
      update(name, [...stringList(values[name]), nextValue])
  }

  return (
    <section
      aria-label="Document metadata"
      className="review-document-frontmatter review-document-issue-frontmatter"
    >
      <h2>FRONTMATTER</h2>
      <label className="review-document-issue-title">
        <span>Title</span>
        <textarea
          aria-label="Issue title"
          disabled={disabled}
          value={title}
          onChange={(event) => {
            update('title', event.currentTarget.value)
          }}
        />
      </label>
      <dl className="review-document-issue-fields">
        <dt>Labels</dt>
        <dd className="review-document-issue-labels">
          <div className="review-document-issue-chips">
            {labels.map((label, index) => (
              <span
                key={`${label}-${String(index)}`}
                className="review-document-issue-chip"
              >
                {label}
                <button
                  aria-label={`Remove label ${label}`}
                  disabled={disabled}
                  type="button"
                  onClick={() => {
                    update(
                      'labels',
                      labels.filter((_, i) => i !== index),
                    )
                  }}
                >
                  ×
                </button>
              </span>
            ))}
            <input
              aria-label="Add label"
              className="review-document-issue-add-field review-document-issue-add-field--label"
              disabled={disabled}
              placeholder="Add label"
              onBlur={(event) => {
                addListValue('labels', event.currentTarget.value)
                event.currentTarget.value = ''
              }}
              onKeyDown={(event) => {
                if (event.key !== 'Enter') return
                event.preventDefault()
                addListValue('labels', event.currentTarget.value)
                event.currentTarget.value = ''
              }}
            />
          </div>
        </dd>
        <dt>Assignees</dt>
        <dd className="review-document-issue-assignees">
          <div className="review-document-issue-chips">
            {assignees.map((assignee, index) => (
              <span
                key={`${assignee}-${String(index)}`}
                className="review-document-issue-chip review-document-issue-chip--assignee"
              >
                <span aria-hidden="true">{initials(assignee)}</span>
                {assignee}
                <button
                  aria-label={`Remove assignee ${assignee}`}
                  disabled={disabled}
                  type="button"
                  onClick={() => {
                    update(
                      'assignees',
                      assignees.filter((_, i) => i !== index),
                    )
                  }}
                >
                  ×
                </button>
              </span>
            ))}
            <AddIssueValue
              buttonLabel="+ Add assignee"
              inputLabel="Add assignee"
              placeholder="Add assignee"
              disabled={disabled}
              onAdd={(value) => {
                addListValue('assignees', value)
              }}
            />
          </div>
        </dd>
        <dt>Parent issue</dt>
        <dd className="review-document-issue-parent">
          {parentIssue.length > 0 ? (
            <div className="review-document-issue-reference">
              <IssueReference reference={parentIssue} />
              <button
                aria-label="Clear parent issue"
                disabled={disabled}
                type="button"
                onClick={() => {
                  update('parentIssue', null)
                }}
              >
                ×
              </button>
            </div>
          ) : null}
          <AddIssueValue
            buttonLabel="Set parent"
            inputLabel="Set parent issue"
            placeholder="owner/repo#123"
            parent
            disabled={disabled}
            onAdd={(value) => {
              update('parentIssue', value)
            }}
          />
        </dd>
        <dt>Sub-issues</dt>
        <dd className="review-document-issue-sub-items">
          {subIssues.map((subIssue, index) => (
            <span
              key={`${subIssue}-${String(index)}`}
              className="review-document-issue-reference"
            >
              <IssueReference reference={subIssue} />
              <button
                aria-label={`Remove sub-issue ${subIssue}`}
                disabled={disabled}
                type="button"
                onClick={() => {
                  update(
                    'subIssues',
                    subIssues.filter((_, i) => i !== index),
                  )
                }}
              >
                ×
              </button>
            </span>
          ))}
          <AddIssueValue
            buttonLabel="+ Add sub-issue"
            inputLabel="Add sub-issue"
            placeholder="owner/repo#123"
            disabled={disabled}
            onAdd={(value) => {
              addListValue('subIssues', value)
            }}
          />
        </dd>
      </dl>
    </section>
  )
}

function initials(author: string) {
  return author.replace(/^@/, '').slice(0, 1).toUpperCase()
}

function readableTime(value: string | null) {
  if (value === null || value.length === 0) return ''
  const timestamp = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})/.exec(value)
  const date = timestamp?.[1]
  const time = timestamp?.[2]
  return date === undefined || time === undefined ? value : `${date} ${time}`
}

function resizeTextarea(textarea: HTMLTextAreaElement) {
  const borderHeight = textarea.offsetHeight - textarea.clientHeight
  textarea.style.height = 'auto'
  textarea.style.height = `${String(textarea.scrollHeight + borderHeight)}px`
}

function ReviewThreadReplyTextarea({
  canSave,
  path,
  reply,
  onChange,
}: {
  canSave: boolean
  path: string
  reply: string
  onChange: (reply: string) => void
}) {
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  useLayoutEffect(() => {
    const textarea = textareaRef.current
    if (textarea === null || typeof ResizeObserver === 'undefined') return

    const resizeObserver = new ResizeObserver(() => {
      resizeTextarea(textarea)
    })
    resizeObserver.observe(textarea)
    return () => {
      resizeObserver.disconnect()
    }
  }, [])

  useLayoutEffect(() => {
    const textarea = textareaRef.current
    if (textarea !== null) resizeTextarea(textarea)
  }, [reply])

  return (
    <textarea
      aria-label={`Your reply to ${path}`}
      disabled={!canSave}
      ref={textareaRef}
      rows={Math.max(1, reply.split('\n').length)}
      value={reply}
      onChange={(event) => {
        onChange(event.currentTarget.value)
      }}
    />
  )
}

export function ReviewDocumentPreview({
  summary,
  body,
  frontmatter = null,
  mode = 'body',
  disabled = false,
  onFrontmatterChange = () => undefined,
  canSave = true,
  onBodyChange = () => undefined,
}: ReviewDocumentPreviewProps) {
  if (mode === 'frontmatter') {
    return summary?.kind === 'issue_body' ? (
      <IssueFrontmatter
        frontmatter={frontmatter}
        disabled={disabled}
        onChange={onFrontmatterChange}
      />
    ) : null
  }

  if (summary?.kind === 'issue_comment') {
    const comment = parseIssueComment(body)
    return (
      <MarkdownMessage className="review-document-issue-comment-body">
        {comment.body}
      </MarkdownMessage>
    )
  }

  if (summary?.kind === 'review_replies') {
    const threads = parseReviewThreads(body)
    if (threads.length === 0) {
      const messages = parseReviewMessages(body)
      return (
        <MarkdownMessage className="review-document-body">
          {messages.length > 0 ? messages.join('\n\n') : body}
        </MarkdownMessage>
      )
    }
    return (
      <div className="review-document-review-threads">
        {threads.map((thread) => (
          <article
            key={thread.id}
            aria-label={thread.path}
            className="review-document-thread"
          >
            <header className="review-document-thread-header">
              <File aria-hidden="true" size={14} />
              <span className="review-document-thread-path">
                {thread.path}
                {thread.line === null ? '' : `:${String(thread.line)}`}
              </span>
              <label className="review-document-thread-resolve">
                <input
                  checked={thread.resolved}
                  disabled={!canSave}
                  type="checkbox"
                  onChange={(event) => {
                    onBodyChange(
                      updateReviewThread(body, thread.id, {
                        resolved: event.currentTarget.checked,
                      }),
                    )
                  }}
                />
                Resolve
              </label>
            </header>
            {thread.diff.length === 0 ? null : (
              <div aria-label="Diff" className="review-document-thread-diff">
                {thread.diff.map((line, index) => (
                  <div
                    key={`${String(index)}-${line.text}`}
                    className={`review-document-diff-line review-document-diff-line--${line.kind}${line.focused ? ' review-document-diff-line--focused' : ''}`}
                  >
                    {line.text}
                  </div>
                ))}
              </div>
            )}
            <div className="review-document-thread-content">
              {thread.comments.map((comment, index) => (
                <div
                  key={`${comment.author}-${comment.createdAt}-${String(index)}`}
                  className="review-document-thread-comment"
                >
                  <span aria-hidden="true" className="review-document-avatar">
                    {initials(comment.author)}
                  </span>
                  <div className="review-document-thread-comment-card">
                    <p className="review-document-thread-comment-meta">
                      <strong>{comment.author}</strong>
                      <span aria-hidden="true"> · </span>
                      <time dateTime={comment.createdAt}>
                        {readableTime(comment.createdAt)}
                      </time>
                    </p>
                    <MarkdownMessage className="review-document-thread-comment-body">
                      {comment.body}
                    </MarkdownMessage>
                  </div>
                </div>
              ))}
              <label className="review-document-thread-reply">
                <span>Your reply</span>
                <ReviewThreadReplyTextarea
                  canSave={canSave}
                  path={thread.path}
                  reply={thread.reply}
                  onChange={(reply) => {
                    onBodyChange(
                      updateReviewThread(body, thread.id, {
                        reply,
                      }),
                    )
                  }}
                />
              </label>
            </div>
          </article>
        ))}
      </div>
    )
  }

  return (
    <MarkdownMessage className="review-document-body">{body}</MarkdownMessage>
  )
}
