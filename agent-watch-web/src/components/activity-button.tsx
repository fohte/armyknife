import '#components/activity-button.css'

import { Activity } from 'lucide-react'

import {
  isAsyncActionFailureStatus,
  useOptionalAsyncActionFeedback,
} from '#async-action-feedback'

type ActivityButtonProps = {
  drawer?: boolean
  activityCount?: number
  failed?: boolean
  active?: boolean
  onOpenActivity?: () => void
}

export function ActivityButton({
  drawer = false,
  activityCount,
  failed: failedOverride,
  active,
  onOpenActivity,
}: ActivityButtonProps) {
  const feedback = useOptionalAsyncActionFeedback()
  const actions = feedback?.actions ?? []
  const count = activityCount ?? actions.length
  if (count === 0) return null
  const activityOpen = active ?? feedback?.activityOpen ?? false
  const openActivity = onOpenActivity ?? (() => feedback?.openActivity())
  const running = actions.filter(
    (action) => action.status === 'merging' || action.status === 'closing',
  ).length
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
      aria-label={`Activity, ${label}`}
      className={`async-action-activity-button${drawer ? ' is-drawer' : ''}${activityOpen ? ' is-open' : ''}`}
      type="button"
      onClick={() => {
        openActivity()
      }}
    >
      <Activity aria-hidden="true" size={drawer ? 16 : 14} />
      <span>Activity</span>
      <span
        className={`async-action-activity-state${failed ? ' is-error' : running > 0 ? ' is-running' : ''}`}
      >
        {String(count)}
      </span>
    </button>
  )
}
