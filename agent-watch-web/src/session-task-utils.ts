import type { SessionTask } from '#types'

export function getSessionTasks<TTask extends Pick<SessionTask, 'id'>>({
  task,
  tasks,
}: {
  task: TTask | null
  tasks?: readonly TTask[]
}): readonly TTask[] {
  return tasks ?? (task === null ? [] : [task])
}
