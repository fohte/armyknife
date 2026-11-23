import { useEffect, useState } from 'react'

import {
  isReviewDocumentSummaryList,
  type ReviewDocumentSummary,
} from '#components/conversation-pane-response'
import { requestJson } from '#components/conversation-requests'
import type { Session } from '#types'

type EditorReviewDocument = {
  sessionId: string
  id: string
  summary: ReviewDocumentSummary | null
}

export function useEditorReviewDocument(session: Session) {
  const [editorDocument, setEditorDocument] =
    useState<EditorReviewDocument | null>(null)
  const reviewDocumentId = session.reviewDocumentId

  useEffect(() => {
    if (reviewDocumentId == null) {
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
          setEditorDocument({
            sessionId: session.id,
            id: reviewDocumentId,
            summary:
              documents.find(
                (document) =>
                  document.id === reviewDocumentId && document.isEditor,
              ) ?? null,
          })
        }
      },
      () => {
        if (current) {
          setEditorDocument({
            sessionId: session.id,
            id: reviewDocumentId,
            summary: null,
          })
        }
      },
    )
    return () => {
      current = false
    }
  }, [reviewDocumentId, session.id])

  return editorDocument?.sessionId === session.id &&
    editorDocument.id === reviewDocumentId
    ? editorDocument.summary
    : null
}
