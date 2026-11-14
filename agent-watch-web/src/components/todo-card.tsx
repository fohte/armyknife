import '#components/todo-card.css'

import { Button } from '@fohte/ui/button'
import type { ClipboardEvent } from 'react'
import { useEffect, useId, useMemo, useState } from 'react'

import {
  AttachmentPreviewList,
  handleAttachmentPaste,
} from '#components/attachment-preview-list'
import { CollapsibleText } from '#components/collapsible-text'
import { MarkdownMessage } from '#components/markdown-message'
import type { EncodedAttachment } from '#components/use-attachments'
import { useAttachments } from '#components/use-attachments'

type TodoCardProps = {
  seq?: number
  sessionId?: string
  title: string
  detail: string | null
  resolution: 'completed' | 'rejected' | 'withdrawn' | null
  defaultComment?: string
  onResolve: (
    resolution: 'completed' | 'rejected',
    comment: string | null,
    attachments?: readonly EncodedAttachment[],
  ) => Promise<boolean>
}

export function TodoCard({
  seq = 0,
  sessionId = 'todo',
  title,
  detail,
  resolution,
  defaultComment = '',
  onResolve,
}: TodoCardProps) {
  const commentId = useId()
  const attachmentState = useAttachments(`${sessionId}:todo:${String(seq)}`)
  const detailContent = useMemo(
    () =>
      detail === null ? null : <MarkdownMessage>{detail}</MarkdownMessage>,
    [detail],
  )
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

  const resolveTodo = async (nextResolution: 'completed' | 'rejected') => {
    if (
      isResolved ||
      isSubmitting ||
      attachmentState.isProcessingAttachments ||
      attachmentState.hasLimitError
    ) {
      return
    }
    const normalizedComment = comment.trim()
    setIsSubmitting(true)
    try {
      await attachmentState.sendWithAttachments(async (attachments) => {
        const succeeded =
          attachments.length === 0
            ? await onResolve(
                nextResolution,
                normalizedComment === '' ? null : normalizedComment,
              )
            : await onResolve(
                nextResolution,
                normalizedComment === '' ? null : normalizedComment,
                attachments,
              )
        if (succeeded) setResolutionLocally(nextResolution)
        return succeeded
      })
    } finally {
      setIsSubmitting(false)
    }
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
        {resolution === null ? (
          <AttachmentPreviewList attachments={attachmentState.attachments} />
        ) : null}
      </article>
    )
  }

  return (
    <article
      className="chat-todo-card"
      aria-label="User task"
      aria-busy={isSubmitting || attachmentState.isProcessingAttachments}
      data-resolution="pending"
    >
      <h2>To-do · pending</h2>
      <h3>{title}</h3>
      {detailContent !== null ? (
        <CollapsibleText
          className="chat-todo-detail"
          expanded={detailExpanded}
          onExpandedChange={setDetailExpanded}
        >
          {detailContent}
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
        onPaste={(event: ClipboardEvent<HTMLTextAreaElement>) => {
          handleAttachmentPaste(event, attachmentState.onPasteFiles)
        }}
        onChange={(event) => {
          setComment(event.currentTarget.value)
        }}
      />
      <AttachmentPreviewList
        attachments={attachmentState.attachments}
        {...(isSubmitting
          ? {}
          : { onRemove: attachmentState.removeAttachment })}
      />
      {attachmentState.error === null ? null : (
        <p role="alert">{attachmentState.error}</p>
      )}
      <footer className="chat-todo-footer">
        <Button
          type="button"
          variant="plain"
          className="aw-button aw-button--primary"
          disabled={
            isSubmitting ||
            attachmentState.isProcessingAttachments ||
            attachmentState.hasLimitError
          }
          onClick={() => {
            void resolveTodo('completed')
          }}
        >
          Mark done
        </Button>
        <Button
          type="button"
          variant="plain"
          className="aw-button chat-todo-reject"
          disabled={
            isSubmitting ||
            attachmentState.isProcessingAttachments ||
            attachmentState.hasLimitError
          }
          onClick={() => {
            void resolveTodo('rejected')
          }}
        >
          Reject
        </Button>
      </footer>
    </article>
  )
}
