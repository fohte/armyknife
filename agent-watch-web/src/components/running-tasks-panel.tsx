import '#components/running-tasks-panel.css'

import { Result } from 'neverthrow'
import { useEffect, useState } from 'react'

import { BackgroundTaskRowView } from '#components/running-task-row-view'
import { RunningTasksPanelView } from '#components/running-tasks-panel-view'
import type { BackgroundTask } from '#types'

type RunningTasksPanelProps = {
  sessionId: string
  tasks: readonly BackgroundTask[]
}

function isOutputMessage(value: unknown): value is { output: string } {
  return (
    typeof value === 'object' &&
    value !== null &&
    'output' in value &&
    typeof value.output === 'string'
  )
}

export function RunningTasksPanel({
  sessionId,
  tasks,
}: RunningTasksPanelProps) {
  const subagents = tasks.filter((task) => task.kind === 'subagent')
  const backgroundTasks = tasks.filter(
    (task) => task.kind === 'background_task',
  )

  return (
    <RunningTasksPanelView
      subagents={subagents}
      backgroundTaskRows={backgroundTasks.map((task) => (
        <BackgroundTaskRowContainer
          key={task.id}
          sessionId={sessionId}
          task={task}
        />
      ))}
    />
  )
}

function BackgroundTaskRowContainer({
  sessionId,
  task,
}: {
  sessionId: string
  task: BackgroundTask
}) {
  const [isOpen, setIsOpen] = useState(false)
  const [output, setOutput] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!isOpen) return
    const url = `/api/sessions/${encodeURIComponent(sessionId)}/background-tasks/${encodeURIComponent(task.id)}/output/events`
    const eventSource = new EventSource(url)
    const onOpen = () => {
      setError(null)
    }
    const onMessage = (event: Event) => {
      if (!(event instanceof MessageEvent)) return
      const data: unknown = event.data
      if (typeof data !== 'string') return
      const parsed = Result.fromThrowable(
        () => JSON.parse(data) as unknown,
        (error: unknown) => error,
      )()
      if (parsed.isErr() || !isOutputMessage(parsed.value)) {
        setError('Unable to load task output.')
        return
      }
      setOutput(parsed.value.output)
      setError(null)
    }
    const onError = () => {
      setError(
        eventSource.readyState === EventSource.CLOSED
          ? 'Unable to load task output.'
          : 'Connection interrupted. Reconnecting…',
      )
    }
    eventSource.addEventListener('open', onOpen)
    eventSource.addEventListener('message', onMessage)
    eventSource.addEventListener('error', onError)
    return () => {
      eventSource.removeEventListener('open', onOpen)
      eventSource.removeEventListener('message', onMessage)
      eventSource.removeEventListener('error', onError)
      eventSource.close()
    }
  }, [isOpen, sessionId, task.id])

  return (
    <BackgroundTaskRowView
      task={task}
      isOpen={isOpen}
      output={output}
      error={error}
      onToggle={() => {
        setIsOpen((current) => !current)
      }}
    />
  )
}
