import { useState } from 'react'

import { useAsyncActionStarter } from '#async-action-feedback'
import {
  BangCommandButton,
  type CommandStatus,
} from '#components/bang-command-button'
import { BangCommandDialog } from '#components/bang-command-dialog'
import type {
  CommandOutputRefresh,
  CommandOutputStart,
} from '#components/command-result-message'
import { useSessionOperations } from '#session-operation-context'
import { createSessionOperation } from '#session-operations'

type BangCommandActionsProps = {
  commands: readonly string[]
  messageText: string
  sessionTitle: string
  sessionId: string
  mobile?: boolean
  onCommandExecuted?: CommandOutputRefresh
  onCommandStarted?: CommandOutputStart
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
  sessionTitle,
  sessionId,
  mobile = false,
  onCommandExecuted,
  onCommandStarted,
}: BangCommandActionsProps) {
  const { run } = useSessionOperations()
  const startAsyncAction = useAsyncActionStarter(() => undefined)
  const [activeKey, setActiveKey] = useState<string | null>(null)
  const [selectedCommands, setSelectedCommands] = useState<string[]>([])
  const [cwd, setCwd] = useState('')
  const [previewing, setPreviewing] = useState(false)
  const [executing, setExecuting] = useState(false)
  const [statuses, setStatuses] = useState<Record<string, CommandStatus>>({})
  const [messages, setMessages] = useState<Record<string, string>>({})
  const onFailure = (key: string, error: Error) => {
    setPreviewing(false)
    setActiveKey(null)
    setStatuses((current) => ({ ...current, [key]: 'failed' }))
    setMessages((current) => ({ ...current, [key]: error.message }))
  }

  const openDialog = (key: string, selected: string[]) => {
    setActiveKey(key)
    setSelectedCommands(selected)
    setPreviewing(true)
    setStatuses((current) => ({ ...current, [key]: 'running' }))
    const operation = createSessionOperation(sessionId, 'commands/preview', {
      messageText,
      commands: selected,
    })
    operation.match(
      (value) => {
        void run(value).then((result) => {
          result.match(
            (response) => {
              if (!isCommandPreview(response)) {
                onFailure(
                  key,
                  new Error('The server returned an invalid response'),
                )
                return
              }
              setCwd(response.cwd)
              setPreviewing(false)
              setStatuses((current) => ({ ...current, [key]: 'idle' }))
            },
            (error) => {
              onFailure(key, error)
            },
          )
        })
      },
      (error) => {
        onFailure(key, error)
      },
    )
  }

  const confirm = () => {
    if (activeKey === null || executing) return
    const key = activeKey
    setStatuses((current) => ({ ...current, [key]: 'running' }))
    const commands = [...selectedCommands]
    const outputStartIndex = onCommandStarted?.() ?? null
    setExecuting(true)
    startAsyncAction({
      kind: 'command',
      sessionId,
      sessionTitle,
      command: commands.join('\n'),
      ...(outputStartIndex === null ? {} : { timelineIndex: outputStartIndex }),
      run: async () => {
        try {
          const operation = createSessionOperation(
            sessionId,
            'commands/execute',
            {
              messageText,
              commands,
            },
          )
          if (operation.isErr()) {
            onFailure(key, operation.error)
            return { status: 'commandFailed', error: operation.error.message }
          }
          const result = await run(operation.value)
          return await result.match(
            async (response) => {
              if (!isCommandExecution(response)) {
                const error = new Error(
                  'The server returned an invalid response',
                )
                onFailure(key, error)
                return { status: 'commandFailed', error: error.message }
              }
              const commandOutputIndex =
                (await onCommandExecuted?.(commands, outputStartIndex)) ?? null
              setStatuses((current) => ({
                ...current,
                [key]: response.succeeded ? 'succeeded' : 'failed',
              }))
              setMessages((current) => ({
                ...current,
                [key]: response.timedOut
                  ? 'Timed out'
                  : response.succeeded
                    ? 'Completed'
                    : `Failed (exit code ${String(response.exitCode)})`,
              }))
              return response.timedOut
                ? {
                    status: 'commandTimedOut',
                    ...(commandOutputIndex === null
                      ? {}
                      : { commandOutputIndex }),
                  }
                : response.succeeded
                  ? {
                      status: 'ranCommand',
                      ...(commandOutputIndex === null
                        ? {}
                        : { commandOutputIndex }),
                    }
                  : {
                      status: 'commandFailed',
                      exitCode: response.exitCode,
                      ...(commandOutputIndex === null
                        ? {}
                        : { commandOutputIndex }),
                    }
            },
            (error) => {
              onFailure(key, error)
              return { status: 'commandFailed', error: error.message }
            },
          )
        } finally {
          setExecuting(false)
        }
      },
    })
    setActiveKey(null)
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
        mobile={mobile}
        onCancel={() => {
          setActiveKey(null)
        }}
        onConfirm={confirm}
        open={activeKey !== null && !previewing}
      />
    </span>
  )
}
