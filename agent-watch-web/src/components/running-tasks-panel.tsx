import '#components/running-tasks-panel.css'

import { useEffect, useState } from 'react'

import { BackgroundTaskRowView } from '#components/running-task-row-view'
import { RunningTasksPanelView } from '#components/running-tasks-panel-view'
import type { BackgroundTask } from '#types'

type RunningTasksPanelProps = {
  sessionId: string
  tasks: readonly BackgroundTask[]
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
      if (event instanceof MessageEvent && typeof event.data === 'string') {
        setOutput(event.data)
        setError(null)
      }
    }
    const onError = () => {
      setError('Connection interrupted. Reconnecting…')
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
