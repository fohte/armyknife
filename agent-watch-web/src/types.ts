type SessionCategory = 'needs_action' | 'in_progress' | 'done' | (string & {})

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

type ParentSession = {
  id: string
  title: string | null
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
  project: string
  parent: ParentSession | null
  ancestorSessionIds: string[]
  pullRequest: PullRequest | null
}

export type SessionsResponse = {
  revision: number
  sessions: Session[]
}
