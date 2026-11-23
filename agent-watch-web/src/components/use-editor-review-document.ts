import { useEffect, useState } from 'react'

import {
  type EditorReviewDocumentState,
  isReviewDocumentSummaryList,
  type ReviewDocumentSummary,
} from '#components/conversation-pane-response'
import { requestJson } from '#components/conversation-requests'
import type { Session } from '#types'

type EditorReviewDocument = {
  sessionId: string
  reviewDocumentId: string | null
  reviewDocumentCount: number
  state: EditorReviewDocumentState
  summaries: ReviewDocumentSummary[] | null
}

export function useEditorReviewDocument(session: Session) {
  const [editorDocument, setEditorDocument] =
    useState<EditorReviewDocument | null>(null)
  const reviewDocumentId = session.reviewDocumentId ?? null
  const reviewDocumentCount =
    session.reviewDocumentCount ?? Number(reviewDocumentId != null)

  useEffect(() => {
    if (reviewDocumentId == null && reviewDocumentCount === 0) {
      setEditorDocument(null)
      return
    }

    let current = true
    setEditorDocument(null)
    void requestJson(
      `/api/sessions/${encodeURIComponent(session.id)}/review-documents`,
      isReviewDocumentSummaryList,
    ).match(
      (documents) => {
        if (current) {
          const summary = documents.find(
            (document) => document.id === reviewDocumentId && document.isEditor,
          )
          setEditorDocument({
            sessionId: session.id,
            reviewDocumentId,
            reviewDocumentCount,
            state:
              summary === undefined
                ? { status: 'unavailable' }
                : { status: 'ready', summary },
            summaries: documents,
          })
        }
      },
      () => {
        if (current) {
          setEditorDocument({
            sessionId: session.id,
            reviewDocumentId,
            reviewDocumentCount,
            state: { status: 'unavailable' },
            summaries: null,
          })
        }
      },
    )
    return () => {
      current = false
    }
  }, [reviewDocumentCount, reviewDocumentId, session.id])

  return editorDocument?.sessionId === session.id &&
    editorDocument.reviewDocumentId === reviewDocumentId &&
    editorDocument.reviewDocumentCount === reviewDocumentCount
    ? editorDocument
    : null
}
