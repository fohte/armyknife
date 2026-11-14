import type { LucideIcon } from 'lucide-react'
import {
  CircleHelp,
  CirclePause,
  Eye,
  List,
  MessageCircleQuestion,
  ShieldQuestion,
  Signpost,
  Square,
  TriangleAlert,
} from 'lucide-react'

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

export function sessionKindIcon(kind: string): LucideIcon {
  switch (kind) {
    case 'Plan':
      return List
    case 'Question':
      return MessageCircleQuestion
    case 'Approve':
      return ShieldQuestion
    case 'Proposal':
      return Signpost
    case 'Todo':
      return Square
    case 'Error':
      return TriangleAlert
    case 'Review':
      return Eye
    case 'Stopped':
      return CirclePause
    default:
      return CircleHelp
  }
}
