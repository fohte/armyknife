import type { Session } from '#types'

export function formatSessionDecisionCount(
  session: Pick<
    Session,
    'category' | 'decisionCount' | 'unreviewedDecisionCount'
  >,
) {
  const decisionCount = session.decisionCount ?? 0
  const unreviewedDecisionCount = session.unreviewedDecisionCount ?? 0

  if (session.category === 'done' && unreviewedDecisionCount > 0) {
    return `◇ ${String(unreviewedDecisionCount)} decisions to review`
  }
  if (decisionCount === 0) return null

  const label = decisionCount === 1 ? 'decision' : 'decisions'
  return `◇ ${String(decisionCount)} ${label}`
}
