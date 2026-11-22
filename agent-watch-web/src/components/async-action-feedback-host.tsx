import '#components/async-action-feedback-host.css'

import { type RefObject, useEffect } from 'react'

import type { AsyncAction } from '#async-action-feedback'
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
  openInTerminal: (id: number) => void
}

function timeLabel(timestamp: number) {
  const date = new Date(timestamp)
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
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
  openInTerminal,
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
                <p>Merge and Close results appear here.</p>
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
                    time={timeLabel(action.createdAt)}
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
                    onOpenInTerminal={() => {
                      closeActivity()
                      openInTerminal(action.id)
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
