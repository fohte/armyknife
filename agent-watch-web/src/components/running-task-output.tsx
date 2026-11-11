import '#components/running-tasks-panel.css'

type BackgroundTaskOutputProps = {
  id: string
  output: string | null
  error: string | null
}

export function BackgroundTaskOutput({
  id,
  output,
  error,
}: BackgroundTaskOutputProps) {
  return (
    <pre aria-live="polite" className="running-task-output" id={id}>
      {error ?? output ?? 'Loading output…'}
    </pre>
  )
}
