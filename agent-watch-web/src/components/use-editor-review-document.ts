import { useEffect, useState } from 'react'

import {
  type EditorReviewDocumentState,
  isReviewDocumentSummaryList,
} from '#components/conversation-pane-response'
import { requestJson } from '#components/conversation-requests'
import type { Session } from '#types'

type EditorReviewDocument = {
  sessionId: string
  id: string
  state: EditorReviewDocumentState
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
          const summary = documents.find(
            (document) => document.id === reviewDocumentId && document.isEditor,
          )
          setEditorDocument({
            sessionId: session.id,
            id: reviewDocumentId,
            state:
              summary === undefined
                ? { status: 'unavailable' }
                : { status: 'ready', summary },
          })
        }
      },
      () => {
        if (current) {
          setEditorDocument({
            sessionId: session.id,
            id: reviewDocumentId,
            state: { status: 'unavailable' },
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
    ? editorDocument.state
    : null
}
