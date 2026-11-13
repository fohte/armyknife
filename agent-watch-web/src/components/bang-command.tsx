import { useState } from 'react'

import {
  BangCommandButton,
  type CommandStatus,
} from '#components/bang-command-button'
import { BangCommandDialog } from '#components/bang-command-dialog'
import { requestActionJson } from '#session-action-requests'

type BangCommandActionsProps = {
  commands: readonly string[]
  messageText: string
  sessionId: string
  onCommandExecuted?: () => void
}

type CommandPreview = { cwd: string }
type CommandExecution = {
  exitCode: number
  succeeded: boolean
  timedOut: boolean
}

function isCommandPreview(value: unknown): value is CommandPreview {
  return (
    typeof value === 'object' &&
    value !== null &&
    'cwd' in value &&
    typeof value.cwd === 'string'
  )
}

function isCommandExecution(value: unknown): value is CommandExecution {
  return (
    typeof value === 'object' &&
    value !== null &&
    'exitCode' in value &&
    typeof value.exitCode === 'number' &&
    'succeeded' in value &&
    typeof value.succeeded === 'boolean' &&
    'timedOut' in value &&
    typeof value.timedOut === 'boolean'
  )
}

export function BangCommandActions({
  commands,
  messageText,
  sessionId,
  onCommandExecuted,
}: BangCommandActionsProps) {
  const [activeKey, setActiveKey] = useState<string | null>(null)
  const [selectedCommands, setSelectedCommands] = useState<string[]>([])
  const [cwd, setCwd] = useState('')
  const [previewing, setPreviewing] = useState(false)
  const [statuses, setStatuses] = useState<Record<string, CommandStatus>>({})
  const [messages, setMessages] = useState<Record<string, string>>({})
  const [executing, setExecuting] = useState(false)

  const openDialog = (key: string, selected: string[]) => {
    setActiveKey(key)
    setSelectedCommands(selected)
    setPreviewing(true)
    setStatuses((current) => ({ ...current, [key]: 'running' }))
    void requestActionJson(
      sessionId,
      'commands/preview',
      { messageText, commands: selected },
      isCommandPreview,
    ).match(
      (preview) => {
        setCwd(preview.cwd)
        setPreviewing(false)
        setStatuses((current) => ({ ...current, [key]: 'idle' }))
      },
      (error) => {
        setPreviewing(false)
        setActiveKey(null)
        setStatuses((current) => ({ ...current, [key]: 'failed' }))
        setMessages((current) => ({ ...current, [key]: error.message }))
      },
    )
  }

  const confirm = () => {
    if (activeKey === null || executing) return
    const key = activeKey
    setExecuting(true)
    setStatuses((current) => ({ ...current, [key]: 'running' }))
    void requestActionJson(
      sessionId,
      'commands/execute',
      { messageText, commands: selectedCommands },
      isCommandExecution,
    ).match(
      (result) => {
        onCommandExecuted?.()
        setExecuting(false)
        setActiveKey(null)
        setStatuses((current) => ({
          ...current,
          [key]: result.succeeded ? 'succeeded' : 'failed',
        }))
        setMessages((current) => ({
          ...current,
          [key]: result.timedOut
            ? 'Timed out'
            : result.succeeded
              ? 'Completed'
              : `Failed (exit code ${String(result.exitCode)})`,
        }))
      },
      (error) => {
        setExecuting(false)
        setActiveKey(null)
        setStatuses((current) => ({ ...current, [key]: 'failed' }))
        setMessages((current) => ({ ...current, [key]: error.message }))
      },
    )
  }

  const statusFor = (key: string) => statuses[key] ?? 'idle'
  const labelFor = (key: string, label: string) => {
    const status = statusFor(key)
    return status === 'succeeded'
      ? 'Succeeded'
      : status === 'failed'
        ? 'Failed'
        : label
  }

  const busy = previewing || executing
  return (
    <span className="bang-command-actions">
      {commands.map((command, index) => {
        const key = `command-${String(index)}`
        return (
          <BangCommandButton
            key={key}
            label={labelFor(key, `Run command ${String(index + 1)}`)}
            busy={busy}
            {...(messages[key] === undefined ? {} : { message: messages[key] })}
            onClick={() => {
              openDialog(key, [command])
            }}
            status={statusFor(key)}
          />
        )
      })}
      {commands.length > 1 && (
        <BangCommandButton
          label={labelFor('all', 'Run all')}
          busy={busy}
          {...(messages['all'] === undefined
            ? {}
            : { message: messages['all'] })}
          onClick={() => {
            openDialog('all', [...commands])
          }}
          status={statusFor('all')}
        />
      )}
      <BangCommandDialog
        commands={selectedCommands}
        cwd={cwd}
        onCancel={() => {
          setActiveKey(null)
        }}
        onConfirm={confirm}
        open={activeKey !== null && !previewing}
        running={executing}
      />
    </span>
  )
}
