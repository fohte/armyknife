import '#components/todo-card.css'

import { Button } from '@fohte/ui/button'
import { ChevronDown, ChevronUp } from 'lucide-react'
import { useId, useMemo, useState } from 'react'

import {
  AttachmentPreviewList,
  handleAttachmentPaste,
} from '#components/attachment-preview-list'
import { CollapsibleText } from '#components/collapsible-text'
import { FileDropIndicator } from '#components/file-drop-indicator'
import { MarkdownMessage } from '#components/markdown-message'
import type { EncodedAttachment } from '#components/use-attachments'
import { useAttachments } from '#components/use-attachments'
import { cardDraftStorageKey, useDraft } from '#components/use-composer-draft'
import { useFileDrop } from '#components/use-file-drop'
import { WithdrawnCard } from '#components/withdrawn-card'

type TodoCardProps = {
  seq?: number
  sessionId?: string
  title: string
  detail: string | null
  folded?: boolean
  addedFolded?: boolean
  resolution: 'completed' | 'rejected' | 'withdrawn' | null
  defaultComment?: string
  onResolve: (
    resolution: 'completed' | 'rejected',
    comment: string | null,
    attachments?: readonly EncodedAttachment[],
  ) => Promise<boolean>
  onFold: (folded: boolean) => Promise<boolean>
}

export function TodoCard({
  seq = 0,
  sessionId,
  title,
  detail,
  folded = false,
  addedFolded = false,
  resolution,
  defaultComment = '',
  onResolve,
  onFold,
}: TodoCardProps) {
  const [draftSessionId] = useState(() => sessionId ?? crypto.randomUUID())
  const commentId = useId()
  const attachmentState = useAttachments(
    `${sessionId ?? 'todo'}:todo:${String(seq)}`,
  )
  const commentDraft = useDraft(
    cardDraftStorageKey('todo', draftSessionId, seq, 'comment'),
    defaultComment,
  )
  const { draft: comment, updateDraft, clearDraftIfUnchanged } = commentDraft
  const detailContent = useMemo(
    () =>
      detail === null ? null : <MarkdownMessage>{detail}</MarkdownMessage>,
    [detail],
  )
  const [detailExpanded, setDetailExpanded] = useState(false)
  const [resolutionLocally, setResolutionLocally] = useState<
    'completed' | 'rejected' | null
  >(null)
  const [resolutionComment, setResolutionComment] = useState<string | null>(
    null,
  )
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [isFoldPending, setIsFoldPending] = useState(false)
  const { isFileDragActive, ...fileDropHandlers } = useFileDrop(
    attachmentState.onPasteFiles,
    !isSubmitting,
    true,
  )
  if (resolution === 'withdrawn') {
    return <WithdrawnCard kind="To-do" title={title} />
  }

  const currentResolution = resolution ?? resolutionLocally
  const isResolved = currentResolution !== null

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
    const sentDraft = comment
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
        if (succeeded) {
          setResolutionLocally(nextResolution)
          setResolutionComment(normalizedComment)
          clearDraftIfUnchanged(sentDraft)
        }
        return succeeded
      })
    } finally {
      setIsSubmitting(false)
    }
  }

  const foldTodo = async (nextFolded: boolean) => {
    if (isFoldPending) return
    setIsFoldPending(true)
    try {
      await onFold(nextFolded)
    } finally {
      setIsFoldPending(false)
    }
  }

  if (isResolved) {
    const completionComment =
      resolution !== null
        ? defaultComment
        : (resolutionComment ?? comment.trim())
    const labels = {
      completed: ['done', 'Completed user task'],
      rejected: ['rejected', 'Rejected user task'],
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

  if (folded) {
    return (
      <article
        className="chat-todo-card chat-todo-card--folded"
        aria-label="User task, folded"
        data-resolution="pending"
        data-folded="true"
      >
        <Button
          type="button"
          variant="plain"
          className="chat-todo-folded-row cursor-pointer"
          aria-expanded={false}
          disabled={isFoldPending}
          onClick={() => {
            void foldTodo(false)
          }}
        >
          <span className="chat-todo-folded-label">
            <span>To-do · later</span>
            {addedFolded ? <span> · added folded</span> : null}
          </span>
          <span className="chat-todo-folded-title-row">
            <strong className="chat-todo-folded-title">{title}</strong>
            <ChevronDown aria-hidden="true" className="chat-todo-chevron" />
          </span>
        </Button>
      </article>
    )
  }

  return (
    <article
      className="chat-todo-card"
      aria-label="User task"
      aria-busy={
        isSubmitting || isFoldPending || attachmentState.isProcessingAttachments
      }
      data-resolution="pending"
    >
      <header className="chat-todo-heading">
        <h2>To-do · pending</h2>
        <Button
          type="button"
          variant="plain"
          className="chat-todo-fold-button cursor-pointer"
          aria-label="Fold this to-do"
          title="Fold"
          aria-expanded={true}
          disabled={isFoldPending}
          onClick={() => {
            void foldTodo(true)
          }}
        >
          <ChevronUp aria-hidden="true" />
        </Button>
      </header>
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
      {!commentDraft.storageAvailable ? (
        <p className="conversation-error" role="status">
          Browser storage is unavailable. Drafts may not survive a page reload.
        </p>
      ) : null}
      <div className="chat-card-comment-drop-zone">
        {isFileDragActive ? <FileDropIndicator /> : null}
        <textarea
          id={commentId}
          className="chat-todo-comment"
          rows={2}
          value={comment}
          disabled={isSubmitting}
          onPaste={(event) => {
            handleAttachmentPaste(event, attachmentState.onPasteFiles)
          }}
          {...fileDropHandlers}
          onChange={(event) => {
            updateDraft(event.currentTarget.value)
          }}
        />
      </div>
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
