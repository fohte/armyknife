import type { ChatItem, UserContentPart } from '#types'

export type CommandOutputStart = () => number
export type CommandOutputRefresh = (
  commands: readonly string[],
  startIndex: number | null,
) => Promise<number | null> | number | null | undefined

export function isCommandResultText(text: string) {
  return (
    /^Command (?:exited with code \d+|timed out after \d+ seconds) in /.test(
      text,
    ) && text.includes('\nCommand:\n')
  )
}

type UserMessage = Extract<
  ChatItem,
  { type: 'user' | 'queued_user' | 'user_with_content' }
>

export function commandResultText(message: ChatItem): string | null {
  if (
    message.type !== 'user' &&
    message.type !== 'queued_user' &&
    message.type !== 'user_with_content'
  ) {
    return null
  }

  const content: UserContentPart[] =
    message.type === 'user'
      ? [{ type: 'text', text: message.text }]
      : message.content
  const text = content
    .filter(
      (part): part is Extract<UserContentPart, { type: 'text' }> =>
        part.type === 'text',
    )
    .map((part) => part.text)
    .join('\n')

  return isCommandResultText(text) ? text : null
}

export function commandResultMatchesCommands(
  message: ChatItem,
  commands: readonly string[],
): message is UserMessage {
  const text = commandResultText(message)
  return (
    text !== null && commands.every((command) => text.includes(`$ ${command}`))
  )
}
