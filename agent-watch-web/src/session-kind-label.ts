import type { Session } from '#types'

export function sessionKindTone(session: Session) {
  if (session.kind === 'Error') return 'error'
  return session.category.replace('_', '-')
}

export function sessionKindLabel(session: Session) {
  if (session.kind !== null) return session.kind
  switch (session.category) {
    case 'needs_action':
      return 'Needs action'
    case 'in_progress':
      return 'Working'
    case 'later':
      return 'Later'
    case 'done':
      return 'Done'
    default:
      return session.category.replace('_', ' ')
  }
}
