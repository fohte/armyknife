import type { Session } from '#types'

export function getConversationCritUrl(
  session: Pick<Session, 'critUrl' | 'webUis'>,
) {
  const registeredCritUrl = session.webUis?.find(
    (webUi) => webUi.name === 'crit' && webUi.url.trim() !== '',
  )?.url
  if (registeredCritUrl !== undefined) return registeredCritUrl

  const legacyCritUrl = session.critUrl
  return legacyCritUrl != null && legacyCritUrl.trim() !== ''
    ? legacyCritUrl
    : null
}

export function getPendingCritReviewUrl(session: Session) {
  return session.category === 'needs_action'
    ? getConversationCritUrl(session)
    : null
}
