import { act } from 'react'
import { vi } from 'vitest'

export async function copyCommandFromCard(
  container: ParentNode,
  command: string,
) {
  const writeText = vi.fn(() => Promise.resolve())
  vi.stubGlobal('navigator', { clipboard: { writeText } })

  await act(async () => {
    const { highlightCode } = await import('./syntax-highlighter')
    await highlightCode(command, 'bash')
  })

  const copyButton = container.querySelector<HTMLButtonElement>(
    '.syntax-code-block-copy-button',
  )
  const copyButtonLabel = copyButton?.getAttribute('aria-label') ?? null
  await act(async () => {
    copyButton?.click()
    await Promise.resolve()
  })

  return {
    copyButtonLabel,
    copiedCommands: writeText.mock.calls,
  }
}
