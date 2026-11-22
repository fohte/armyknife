import '#components/running-tasks-panel.css'

import { useEffect, useState } from 'react'

import { requestText } from '#components/conversation-requests'
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
    let active = true
    let timeout: number | null = null
    let requestPending = false
    const poll = () => {
      if (requestPending) return
      requestPending = true
      const url = `/api/sessions/${encodeURIComponent(sessionId)}/background-tasks/${encodeURIComponent(task.id)}/output`
      void requestText(url)
        .match(
          (nextOutput) => {
            if (!active) return
            setOutput(nextOutput)
            setError(null)
          },
          (requestError) => {
            if (!active) return
            setError(requestError.message)
          },
        )
        .then(() => {
          requestPending = false
          if (active) timeout = window.setTimeout(poll, 1_000)
        })
    }
    poll()
    return () => {
      active = false
      if (timeout !== null) window.clearTimeout(timeout)
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
