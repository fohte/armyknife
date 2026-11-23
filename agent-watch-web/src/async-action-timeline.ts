import type { AsyncAction } from '#async-action-feedback'

export type ShiftMergeTimelineIndicesOptions = {
  sessionId: string
  from: number
  through: number
  offset: number
}

export function shiftMergeTimelineIndices(
  actions: AsyncAction[],
  { sessionId, from, through, offset }: ShiftMergeTimelineIndicesOptions,
): AsyncAction[] {
  if (offset === 0) return actions
  return actions.map((action) => {
    const timelineIndex = action.timelineIndex
    return action.kind === 'merge' &&
      action.sessionId === sessionId &&
      timelineIndex !== undefined &&
      timelineIndex >= from &&
      timelineIndex <= through
      ? { ...action, timelineIndex: timelineIndex + offset }
      : action
  })
}
