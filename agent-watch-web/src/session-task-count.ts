import type { SessionTask } from '#types'

export function formatLinkedTaskCount(
  tasks: readonly SessionTask[] | undefined,
) {
  const taskCount = tasks?.length ?? 1
  return taskCount > 1 ? ` +${String(taskCount - 1)}` : ''
}
