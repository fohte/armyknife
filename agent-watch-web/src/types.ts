type SessionCategory =
  'needs_action' | 'later' | 'in_progress' | 'done' | (string & {})

type SessionKind =
  | 'Question'
  | 'Approve'
  | 'Review'
  | 'Error'
  | 'Plan'
  | 'Stopped'
  | (string & {})

type SessionTask = {
  id: string
  number: number
  title: string
  priority: number | null
}

type PullRequest = {
  number: number
  state: 'Open' | 'Closed' | 'Merged' | (string & {})
}

export type Session = {
  id: string
  category: SessionCategory
  categoryWhenNotLater?: SessionCategory
  kind: SessionKind | null
  title: string
  preview: string | null
  elapsedSeconds: number
  unread: boolean
  task: SessionTask | null
  repo: string
  pullRequest: PullRequest | null
  laterUntil?: string | null
  isToday?: boolean
}

export type SessionsResponse = {
  revision: number
  sessions: Session[]
}
