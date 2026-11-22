import '#components/activity-button.css'

import { Activity } from 'lucide-react'

import {
  isAsyncActionFailureStatus,
  useOptionalAsyncActionFeedback,
} from '#async-action-feedback'

type ActivityButtonProps = {
  drawer?: boolean
  activityCount?: number
  isRunning?: boolean
  failed?: boolean
  active?: boolean
  onOpenActivity?: (anchor: HTMLButtonElement) => void
}

export function ActivityButton({
  drawer = false,
  activityCount,
  isRunning: runningOverride,
  failed: failedOverride,
  active,
  onOpenActivity,
}: ActivityButtonProps) {
  const feedback = useOptionalAsyncActionFeedback()
  const actions = feedback?.actions ?? []
  const activityOpen = active ?? feedback?.activityOpen ?? false
  const failedCount = actions.filter((action) =>
    isAsyncActionFailureStatus(action.status),
  ).length
  const runningCount = actions.filter(
    (action) => action.status === 'merging' || action.status === 'closing',
  ).length
  const running =
    runningOverride === undefined ? runningCount : runningOverride ? 1 : 0
  const count =
    activityCount ??
    (failedCount > 0 ? failedCount : running > 0 ? running : actions.length)
  const failed =
    failedOverride ??
    actions.some((action) => isAsyncActionFailureStatus(action.status))
  const label = failed
    ? 'failed actions'
    : running > 0
      ? `${String(running)} running`
      : 'results available'
  return (
    <button
      aria-expanded={activityOpen}
      aria-haspopup="dialog"
      aria-label={`Activity, ${count === 0 ? 'no actions yet' : label}`}
      className={`async-action-activity-button${drawer ? ' is-drawer' : ''}${count === 0 ? ' is-empty' : ''}${activityOpen ? ' is-open' : ''}`}
      type="button"
      onClick={(event) => {
        if (onOpenActivity !== undefined) {
          onOpenActivity(event.currentTarget)
        } else {
          feedback?.openActivity(undefined, event.currentTarget)
        }
      }}
    >
      <Activity aria-hidden="true" size={drawer ? 16 : 14} />
      <span>Activity</span>
      {count > 0 ? (
        <span
          className={`async-action-activity-state${failed ? ' is-error' : running > 0 ? ' is-running' : ''}`}
        >
          {String(count)}
        </span>
      ) : null}
    </button>
  )
}
