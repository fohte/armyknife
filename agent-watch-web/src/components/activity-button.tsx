import '#components/activity-button.css'

import { Activity } from 'lucide-react'

import {
  isAsyncActionFailureStatus,
  useOptionalAsyncActionFeedback,
} from '#async-action-feedback'

type ActivityButtonProps = {
  drawer?: boolean
  iconOnly?: boolean
  activityCount?: number
  failed?: boolean
  active?: boolean
  onOpenActivity?: () => void
}

export function ActivityButton({
  drawer = false,
  iconOnly = false,
  activityCount,
  failed: failedOverride,
  active,
  onOpenActivity,
}: ActivityButtonProps) {
  const feedback = useOptionalAsyncActionFeedback()
  const actions = feedback?.actions ?? []
  const count = activityCount ?? actions.length
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
      aria-label={`Activity, ${count === 0 ? 'no actions yet' : label}`}
      className={`async-action-activity-button${drawer ? ' is-drawer' : ''}${iconOnly ? ' is-icon-only' : ''}${count === 0 ? ' is-empty' : ''}${activityOpen ? ' is-open' : ''}`}
      type="button"
      onClick={() => {
        openActivity()
      }}
    >
      <Activity aria-hidden="true" size={iconOnly ? 18 : drawer ? 16 : 14} />
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
