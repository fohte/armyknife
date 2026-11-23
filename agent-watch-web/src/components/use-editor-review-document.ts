import { useCallback, useEffect, useState } from 'react'

import {
  type EditorReviewDocumentState,
  isReviewDocumentSummaryList,
} from '#components/conversation-pane-response'
import { requestJson } from '#components/conversation-requests'
import type { ReviewDocumentSummariesState } from '#components/review-document-summary'
import type { Session } from '#types'

type EditorReviewDocument = {
  sessionId: string
  reviewDocumentId: string | null
  reviewDocumentCount: number
  state: EditorReviewDocumentState
  summaries: ReviewDocumentSummariesState | null
}

export function useEditorReviewDocument(session: Session) {
  const [editorDocument, setEditorDocument] =
    useState<EditorReviewDocument | null>(null)
  const [summaryRequest, setSummaryRequest] = useState(0)
  const reviewDocumentId = session.reviewDocumentId ?? null
  const reviewDocumentCount =
    session.reviewDocumentCount ?? Number(reviewDocumentId != null)
  const retrySummaries = useCallback(() => {
    setSummaryRequest((request) => request + 1)
  }, [])

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
            summaries: { status: 'ready', summaries: documents },
          })
        }
      },
      (error) => {
        if (current) {
          setEditorDocument({
            sessionId: session.id,
            reviewDocumentId,
            reviewDocumentCount,
            state: { status: 'unavailable' },
            summaries: { status: 'unavailable', error: error.message },
          })
        }
      },
    )
    return () => {
      current = false
    }
  }, [reviewDocumentCount, reviewDocumentId, session.id, summaryRequest])

  const currentEditorDocument =
    editorDocument?.sessionId === session.id &&
    editorDocument.reviewDocumentId === reviewDocumentId &&
    editorDocument.reviewDocumentCount === reviewDocumentCount
      ? editorDocument
      : null
  return currentEditorDocument === null
    ? null
    : { ...currentEditorDocument, retrySummaries }
}
