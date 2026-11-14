import '#components/todo-card.css'

import { Button } from '@fohte/ui/button'
import { useEffect, useId, useState } from 'react'

import { CollapsibleText } from '#components/collapsible-text'
import { MarkdownMessage } from '#components/markdown-message'

type TodoCardProps = {
  title: string
  detail: string | null
  resolution: 'completed' | 'rejected' | 'withdrawn' | null
  defaultComment?: string
  onResolve: (
    resolution: 'completed' | 'rejected',
    comment: string | null,
  ) => Promise<boolean>
}

export function TodoCard({
  title,
  detail,
  resolution,
  defaultComment = '',
  onResolve,
}: TodoCardProps) {
  const commentId = useId()
  const [comment, setComment] = useState(defaultComment)
  const [detailExpanded, setDetailExpanded] = useState(false)
  const [resolutionLocally, setResolutionLocally] = useState<
    'completed' | 'rejected' | null
  >(null)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const currentResolution = resolution ?? resolutionLocally
  const isResolved = currentResolution !== null

  useEffect(() => {
    if (resolution !== null) setComment(defaultComment)
  }, [resolution, defaultComment])

  const resolveTodo = (nextResolution: 'completed' | 'rejected') => {
    if (isResolved || isSubmitting) return
    const normalizedComment = comment.trim()
    setIsSubmitting(true)
    void onResolve(
      nextResolution,
      normalizedComment === '' ? null : normalizedComment,
    )
      .then((succeeded) => {
        if (succeeded) setResolutionLocally(nextResolution)
      })
      .finally(() => {
        setIsSubmitting(false)
      })
  }

  if (isResolved) {
    const completionComment =
      resolution !== null ? defaultComment : comment.trim()
    const labels = {
      completed: ['done', 'Completed user task'],
      rejected: ['rejected', 'Rejected user task'],
      withdrawn: ['withdrawn', 'Withdrawn user task'],
    } as const
    const [status, ariaLabel] = labels[currentResolution]

    return (
      <article
        className="chat-todo-card"
        aria-label={ariaLabel}
        data-resolution={currentResolution}
      >
        <h2>To-do · {status}</h2>
        <h3>{title}</h3>
        {completionComment !== '' ? (
          <MarkdownMessage className="chat-todo-completed-comment">
            {completionComment}
          </MarkdownMessage>
        ) : null}
      </article>
    )
  }

  return (
    <article
      className="chat-todo-card"
      aria-label="User task"
      aria-busy={isSubmitting}
      data-resolution="pending"
    >
      <h2>To-do · pending</h2>
      <h3>{title}</h3>
      {detail !== null ? (
        <CollapsibleText
          className="chat-todo-detail"
          expanded={detailExpanded}
          onExpandedChange={setDetailExpanded}
        >
          <MarkdownMessage>{detail}</MarkdownMessage>
        </CollapsibleText>
      ) : null}
      <label className="chat-todo-comment-label" htmlFor={commentId}>
        Comment (optional)
      </label>
      <textarea
        id={commentId}
        className="chat-todo-comment"
        rows={2}
        value={comment}
        disabled={isSubmitting}
        onChange={(event) => {
          setComment(event.currentTarget.value)
        }}
      />
      <footer className="chat-todo-footer">
        <Button
          type="button"
          variant="plain"
          className="aw-button aw-button--primary"
          disabled={isSubmitting}
          onClick={() => {
            resolveTodo('completed')
          }}
        >
          Mark done
        </Button>
        <Button
          type="button"
          variant="plain"
          className="aw-button chat-todo-reject"
          disabled={isSubmitting}
          onClick={() => {
            resolveTodo('rejected')
          }}
        >
          Reject
        </Button>
      </footer>
    </article>
  )
}
