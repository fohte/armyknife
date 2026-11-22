export function scrollToConversationStatusCard(
  container: HTMLElement | null,
  isPermissionRequest: boolean,
) {
  const cardSelector = isPermissionRequest
    ? '.chat-permission-card:not(.chat-permission-denied-card)'
    : '.chat-question-card[data-has-unanswered-question="true"]'
  const cards = container?.querySelectorAll<HTMLElement>(cardSelector)
  cards?.[cards.length - 1]?.scrollIntoView({
    behavior: 'smooth',
    block: 'center',
  })
}
