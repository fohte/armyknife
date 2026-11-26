import type { Dispatch, SetStateAction } from 'react'

import {
  type AsyncActionOutcome,
  useOptionalAsyncActionFeedback,
} from '#async-action-feedback'
import { type ReviewDocumentResponse } from '#components/conversation-pane-response'
import { reviewDocumentActionLabel } from '#components/review-document-action-label'
import type { ReviewDocumentSummary } from '#components/review-document-summary'
import { useSessionOperations } from '#session-operation-context'
import type {
  ExpectedPullRequestTarget,
  ReviewDocumentAction,
} from '#session-operation-types'
import { createSessionOperation } from '#session-operations'
import type { JsonValue } from '#types'

export type ReviewDocumentState = {
  sessionId: string
  reviewDocumentId: string
  document: ReviewDocumentResponse | null
  error: string | null
  loading: boolean
  saveStatus: 'idle' | 'saving' | 'saved' | 'error' | 'finishing' | 'submitting'
  saveError: string | null
  pendingAction?: ReviewDocumentAction | undefined
  unsavedChanges: boolean
  view: 'edit' | 'preview'
}

export type ReviewDocumentStates = Record<string, ReviewDocumentState>

export function requestReviewDocumentAction(
  sessionId: string,
  document: ReviewDocumentResponse,
  summary: ReviewDocumentSummary,
  action: 'save' | 'finish' | ReviewDocumentAction,
  setState: Dispatch<SetStateAction<ReviewDocumentStates>>,
  run: ReturnType<typeof useSessionOperations>['run'],
  sessionTitle: string,
  feedback: ReturnType<typeof useOptionalAsyncActionFeedback>,
  frontmatter = document.frontmatter,
) {
  const savedRevision = (value: unknown) =>
    typeof value === 'object' &&
    value !== null &&
    'revision' in value &&
    typeof value.revision === 'string'
      ? value.revision
      : null
  setState((current) => {
    const state = current[document.id]
    if (state === undefined || state.sessionId !== sessionId) return current
    return {
      ...current,
      [document.id]: {
        ...state,
        saveStatus:
          action === 'save'
            ? 'saving'
            : action === 'finish'
              ? 'finishing'
              : 'submitting',
        saveError: null,
        ...(action === 'save' || action === 'finish'
          ? { pendingAction: undefined }
          : { pendingAction: action }),
      },
    }
  })

  const onSuccess = (result: unknown) => {
    const revision = savedRevision(result)
    setState((current) => {
      const state = current[document.id]
      if (state === undefined || state.sessionId !== sessionId) return current
      return {
        ...current,
        [document.id]: {
          ...state,
          ...(state.document === null || revision === null
            ? {}
            : { document: { ...state.document, revision } }),
          saveStatus: 'saved',
          saveError: null,
          unsavedChanges: false,
        },
      }
    })
  }
  const onError = (error: Error) => {
    setState((current) => {
      const state = current[document.id]
      return state === undefined || state.sessionId !== sessionId
        ? current
        : {
            ...current,
            [document.id]: {
              ...state,
              ...('revision' in error &&
              typeof error.revision === 'string' &&
              state.document !== null
                ? {
                    document: {
                      ...state.document,
                      revision: error.revision,
                    },
                  }
                : {}),
              saveStatus: 'error',
              saveError: error.message,
              pendingAction: undefined,
            },
          }
    })
  }
  const operationName =
    action === 'save' || action === 'finish' ? action : 'action'
  const operation = createSessionOperation(
    sessionId,
    `review-document/${operationName}`,
    {
      id: document.id,
      revision: document.revision,
      frontmatter,
      body: document.body,
      ...(action === 'save' || action === 'finish'
        ? {}
        : {
            action,
            ...(action === 'create_pr'
              ? {
                  expectedPullRequestTarget: expectedPullRequestTarget(summary),
                }
              : {}),
          }),
    },
  )
  const submit = async (): Promise<AsyncActionOutcome> =>
    new Promise((resolve) => {
      operation.match(
        (value) => {
          void run(value).then((result) => {
            result.match(
              (response) => {
                onSuccess(response)
                resolve({ status: 'reviewDocumentActionSucceeded' })
              },
              (error) => {
                onError(error)
                resolve({
                  status: 'reviewDocumentActionFailed',
                  error: error.message,
                  ...(error.retryable === undefined
                    ? {}
                    : { retryable: error.retryable }),
                })
              },
            )
          })
        },
        (error) => {
          onError(error)
          resolve({
            status: 'reviewDocumentActionFailed',
            error: error.message,
          })
        },
      )
    })

  if (action === 'save' || action === 'finish') {
    void submit()
    return
  }
  const runAction = () => {
    setState((current) => {
      const state = current[document.id]
      return state === undefined || state.sessionId !== sessionId
        ? current
        : {
            ...current,
            [document.id]: {
              ...state,
              saveStatus: 'submitting',
              saveError: null,
              pendingAction: action,
            },
          }
    })
    return submit()
  }
  if (feedback === null) {
    void runAction()
    return
  }
  feedback.start({
    kind: 'review_document',
    reviewDocumentAction: directActionLabel(action, summary),
    sessionId,
    sessionTitle,
    run: runAction,
  })
}

function directActionLabel(
  action: ReviewDocumentAction,
  summary: ReviewDocumentSummary,
) {
  return reviewDocumentActionLabel(action, summary)
}

function expectedPullRequestTarget(
  summary: ReviewDocumentSummary,
): ExpectedPullRequestTarget {
  if (summary.target.pullRequestLookupUnavailable === true) {
    return { kind: 'unknown' }
  }
  if (summary.target.pullRequestNumber !== undefined) {
    return { kind: 'existing', number: summary.target.pullRequestNumber }
  }
  return { kind: 'new' }
}

export function frontmatterForReviewFinish(
  frontmatter: JsonValue,
  translate: boolean,
  preserveSubmit = false,
) {
  if (
    typeof frontmatter !== 'object' ||
    frontmatter === null ||
    Array.isArray(frontmatter)
  ) {
    return frontmatter
  }
  const next = { ...frontmatter }
  if (!preserveSubmit && typeof next['submit'] === 'boolean')
    next['submit'] = false
  const steps = next['steps']
  if (typeof steps === 'object' && steps !== null && !Array.isArray(steps)) {
    next['steps'] = {
      ...steps,
      ...(typeof steps['submit'] === 'boolean' ? { submit: false } : {}),
      ...(typeof steps['ready-for-translation'] === 'boolean'
        ? { 'ready-for-translation': translate }
        : {}),
    }
  }
  return next
}
