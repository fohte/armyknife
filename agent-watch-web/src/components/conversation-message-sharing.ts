import type { ChatItem } from '#types'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function areJsonValuesEqual(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true
  if (Array.isArray(left) || Array.isArray(right)) {
    return (
      Array.isArray(left) &&
      Array.isArray(right) &&
      left.length === right.length &&
      left.every((value, index) => areJsonValuesEqual(value, right[index]))
    )
  }
  if (!isRecord(left) || !isRecord(right)) return false

  const leftKeys = Object.keys(left)
  const rightKeys = Object.keys(right)
  return (
    leftKeys.length === rightKeys.length &&
    leftKeys.every(
      (key) =>
        Object.hasOwn(right, key) && areJsonValuesEqual(left[key], right[key]),
    )
  )
}

export function shareUnchangedMessages(
  previous: ChatItem[],
  next: ChatItem[],
  previousStart = 0,
  nextStart = 0,
): ChatItem[] {
  let unchanged = previousStart === nextStart && previous.length === next.length
  const previousByPosition = new Map(
    previous.map((message, index) => [previousStart + index, message]),
  )
  const shared = next.map((message, index) => {
    const previousMessage = previousByPosition.get(nextStart + index)
    if (
      previousMessage !== undefined &&
      areJsonValuesEqual(previousMessage, message)
    ) {
      return previousMessage
    }
    unchanged = false
    return message
  })
  return unchanged ? previous : shared
}

export function reuseEqualValue<T>(previous: T | null, next: T): T {
  return previous !== null && areJsonValuesEqual(previous, next)
    ? previous
    : next
}
