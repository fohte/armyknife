type SessionCategory = 'needs_action' | 'in_progress' | 'done'

type SessionKind =
  'Question' | 'Approve' | 'Review' | 'Error' | 'Plan' | 'Stopped'

type SessionTask = {
  id: string
  number: number
  title: string
  priority: number | null
}

type PullRequest = {
  number: number
  state: 'Open' | 'Closed' | 'Merged'
}

export type Session = {
  id: string
  category: SessionCategory
  kind: SessionKind | null
  title: string
  preview: string | null
  elapsedSeconds: number
  unread: boolean
  task: SessionTask | null
  repo: string
  pullRequest: PullRequest | null
}

export type SessionsResponse = {
  revision: number
  sessions: Session[]
}
