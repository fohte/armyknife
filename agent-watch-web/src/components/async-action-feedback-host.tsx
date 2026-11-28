import '#components/async-action-feedback-host.css'

import { type RefObject, useEffect } from 'react'

import { type AsyncAction, formatAsyncActionTime } from '#async-action-feedback'
import { AsyncActionItem } from '#components/async-action-item'

type AsyncActionFeedbackHostProps = {
  actions: readonly AsyncAction[]
  toastIds: readonly number[]
  toastRef?: RefObject<HTMLDivElement | null>
  activityOpen: boolean
  activityPosition?: { left: number; bottom: number } | null
  toastBottomOffset: number | null
  selectedActionId: number | null
  retry: (id: number) => void
  dismissToast: (id: number) => void
  clearFinished: () => void
  openActivity: (id?: number) => void
  closeActivity: () => void
  closeWithWorktree: (id: number) => void
  discardChanges?: (id: number) => void
  reviewDiscardAgain?: (id: number) => void
  openInTerminal: (id: number) => void
  openSession?: (id: number) => void
  showCommandOutput: (id: number) => void
}

export function AsyncActionFeedbackHost({
  actions,
  toastIds,
  toastRef,
  activityOpen,
  activityPosition = null,
  toastBottomOffset,
  selectedActionId,
  retry,
  dismissToast,
  clearFinished,
  openActivity,
  closeActivity,
  closeWithWorktree,
  discardChanges,
  reviewDiscardAgain,
  openInTerminal,
  openSession,
  showCommandOutput,
}: AsyncActionFeedbackHostProps) {
  const ordered = [...actions].sort(
    (left, right) => right.createdAt - left.createdAt,
  )
  const selectedAction =
    selectedActionId === null
      ? null
      : (actions.find((action) => action.id === selectedActionId) ?? null)

  useEffect(() => {
    if (!activityOpen) return

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      event.stopPropagation()
      closeActivity()
    }

    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [activityOpen, closeActivity])

  return (
    <>
      <div
        ref={toastRef}
        className="async-action-toast-stack"
        aria-live="polite"
        style={
          toastBottomOffset === null
            ? undefined
            : {
                bottom: `${String(toastBottomOffset)}px`,
                maxHeight: `calc(100dvh - ${String(toastBottomOffset)}px - 16px)`,
              }
        }
      >
        {toastIds.map((id) => {
          const action = actions.find((candidate) => candidate.id === id)
          return action === undefined ? null : (
            <AsyncActionItem
              action={action}
              form="toast"
              key={id}
              onRetry={() => {
                retry(id)
              }}
              onDetails={() => {
                openActivity(id)
              }}
              onDismiss={() => {
                dismissToast(id)
              }}
              onCloseKeepWorktree={() => {
                closeWithWorktree(id)
              }}
              onDiscardChanges={() => {
                discardChanges?.(id)
              }}
              onReviewDiscardAgain={() => {
                reviewDiscardAgain?.(id)
              }}
              {...(openSession === undefined
                ? {}
                : {
                    onOpenSession: () => {
                      openSession(id)
                    },
                  })}
              onShowOutput={() => {
                showCommandOutput(id)
              }}
            />
          )
        })}
      </div>
      {activityOpen ? (
        <div
          className={`async-action-activity-backdrop${activityPosition === null ? '' : ' is-anchored'}`}
          onMouseDown={closeActivity}
        >
          <section
            aria-label="Activity"
            aria-modal="true"
            className={`async-action-activity${activityPosition === null ? '' : ' is-anchored'}`}
            role="dialog"
            style={
              activityPosition === null
                ? undefined
                : {
                    left: `${String(activityPosition.left)}px`,
                    bottom: `${String(activityPosition.bottom)}px`,
                  }
            }
            onMouseDown={(event) => {
              event.stopPropagation()
            }}
          >
            <header>
              <h2>Activity</h2>
              {ordered.length > 0 ? (
                <button type="button" onClick={clearFinished}>
                  Clear finished
                </button>
              ) : null}
            </header>
            {ordered.length === 0 ? (
              <div className="async-action-empty">
                <p>No recent activity.</p>
                <p>
                  Launches, merges, closes, rewinds, and command results appear
                  here.
                </p>
              </div>
            ) : null}
            <div className="async-action-activity-list">
              {ordered.map((action) => (
                <div
                  className={
                    selectedAction?.id === action.id ? 'is-selected' : undefined
                  }
                  key={action.id}
                >
                  <AsyncActionItem
                    action={action}
                    form="row"
                    time={formatAsyncActionTime(action.createdAt)}
                    onRetry={() => {
                      closeActivity()
                      retry(action.id)
                    }}
                    onDetails={() => {
                      openActivity(action.id)
                    }}
                    onCloseKeepWorktree={() => {
                      closeWithWorktree(action.id)
                    }}
                    onDiscardChanges={() => {
                      closeActivity()
                      discardChanges?.(action.id)
                    }}
                    onReviewDiscardAgain={() => {
                      closeActivity()
                      reviewDiscardAgain?.(action.id)
                    }}
                    onOpenInTerminal={() => {
                      closeActivity()
                      openInTerminal(action.id)
                    }}
                    {...(openSession === undefined
                      ? {}
                      : {
                          onOpenSession: () => {
                            closeActivity()
                            openSession(action.id)
                          },
                        })}
                    onShowOutput={() => {
                      closeActivity()
                      showCommandOutput(action.id)
                    }}
                  />
                </div>
              ))}
            </div>
            <button
              className="async-action-activity-close"
              type="button"
              onClick={closeActivity}
            >
              Close
            </button>
          </section>
        </div>
      ) : null}
    </>
  )
}
