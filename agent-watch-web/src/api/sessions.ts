export type SessionCategory = 'needs_action' | 'in_progress' | 'done'

export type SessionKind =
  'Question' | 'Approve' | 'Review' | 'Error' | 'Plan' | 'Stopped'

export type AgentEngine = 'claude' | 'codex'

export type PullRequestState = 'Open' | 'Closed' | 'Merged'

export type SessionTask = {
  id: string
  number: number
  title: string
  priority: number | null
}

export type ParentSession = {
  id: string
  title: string | null
}

export type SessionPullRequest = {
  number: number
  state: PullRequestState
}

export type Session = {
  id: string
  engine: AgentEngine
  category: SessionCategory
  kind: SessionKind | null
  title: string
  preview: string | null
  updatedAt: string
  elapsedSeconds: number
  unread: boolean
  task: SessionTask | null
  repo: string
  parent: ParentSession | null
  pullRequest: SessionPullRequest | null
}

export type SessionsResponse = {
  revision: number
  sessions: Session[]
}
