import type { AsyncAction } from '#async-action-feedback'

export type ShiftActionTimelineIndicesOptions = {
  sessionId: string
  from: number
  through: number
  offset: number
}

export function shiftActionTimelineIndices(
  actions: AsyncAction[],
  { sessionId, from, through, offset }: ShiftActionTimelineIndicesOptions,
): AsyncAction[] {
  if (offset === 0) return actions
  return actions.map((action) => {
    const timelineIndex = action.timelineIndex
    return action.sessionId === sessionId &&
      timelineIndex !== undefined &&
      timelineIndex >= from &&
      timelineIndex <= through
      ? { ...action, timelineIndex: timelineIndex + offset }
      : action
  })
}
