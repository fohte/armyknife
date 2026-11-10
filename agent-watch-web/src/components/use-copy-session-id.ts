import { Result } from 'neverthrow'
import { useEffect, useRef, useState } from 'react'

function copyWithExecCommand(text: string): boolean {
  const textarea = document.createElement('textarea')
  textarea.value = text
  textarea.setAttribute('readonly', '')
  textarea.style.position = 'fixed'
  textarea.style.opacity = '0'
  textarea.style.left = '0'
  textarea.style.top = '0'
  document.body.append(textarea)
  textarea.select()
  textarea.setSelectionRange(0, text.length)

  const copied = Result.fromThrowable(
    () => {
      const execCommand: unknown = Reflect.get(document, 'execCommand')
      if (typeof execCommand !== 'function') return false
      const result: unknown = Reflect.apply(execCommand, document, ['copy'])
      return result === true
    },
    (error: unknown) => error,
  )().match(
    (didCopy) => didCopy,
    () => false,
  )
  textarea.remove()
  return copied
}

function copyTextToClipboard(text: string): Promise<boolean> {
  const clipboard: unknown = Reflect.get(navigator, 'clipboard')
  const writeText: unknown =
    typeof clipboard === 'object' && clipboard !== null
      ? Reflect.get(clipboard, 'writeText')
      : undefined

  if (typeof writeText !== 'function') {
    return Promise.resolve(copyWithExecCommand(text))
  }

  return Promise.resolve()
    .then(() => {
      const result: unknown = Reflect.apply(writeText, clipboard, [text])
      return Promise.resolve(result)
    })
    .then(
      () => true,
      () => Promise.resolve(copyWithExecCommand(text)),
    )
}

export function useCopySessionId(sessionId: string) {
  const [status, setStatus] = useState<'idle' | 'copied' | 'failed'>('idle')
  const resetTimeoutRef = useRef<number | null>(null)

  useEffect(
    () => () => {
      if (resetTimeoutRef.current !== null) {
        window.clearTimeout(resetTimeoutRef.current)
      }
    },
    [],
  )

  const copySessionId = () => {
    void copyTextToClipboard(sessionId).then((copied) => {
      setStatus(copied ? 'copied' : 'failed')
      if (resetTimeoutRef.current !== null) {
        window.clearTimeout(resetTimeoutRef.current)
      }
      resetTimeoutRef.current = window.setTimeout(() => {
        setStatus('idle')
        resetTimeoutRef.current = null
      }, 1500)
    })
  }

  return { status, copySessionId }
}
