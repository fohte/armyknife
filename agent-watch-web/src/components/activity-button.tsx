import '#components/activity-button.css'

import { Activity } from 'lucide-react'

import {
  isAsyncActionFailureStatus,
  isAsyncActionRunningStatus,
  useOptionalAsyncActionFeedback,
} from '#async-action-feedback'

type ActivityButtonProps = {
  drawer?: boolean
  activityCount?: number
  isRunning?: boolean
  failed?: boolean
  active?: boolean
  onOpenActivity?: () => void
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
  const count = activityCount ?? actions.length
  const activityOpen = active ?? feedback?.activityOpen ?? false
  const openActivity = onOpenActivity ?? (() => feedback?.openActivity())
  const running =
    runningOverride === undefined
      ? actions.filter((action) => isAsyncActionRunningStatus(action.status))
          .length
      : runningOverride
        ? 1
        : 0
  const failed =
    failedOverride ??
    actions.some((action) => isAsyncActionFailureStatus(action.status))
  const label =
    running > 0
      ? `${String(running)} running`
      : failed
        ? 'failed actions'
        : 'results available'
  return (
    <button
      aria-expanded={activityOpen}
      aria-haspopup="dialog"
      aria-label={`Activity, ${count === 0 ? 'no actions yet' : label}`}
      className={`async-action-activity-button${drawer ? ' is-drawer' : ''}${count === 0 ? ' is-empty' : ''}${activityOpen ? ' is-open' : ''}`}
      type="button"
      onClick={() => {
        openActivity()
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
