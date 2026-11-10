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

export type Session = {
  id: string
  engine?: 'Claude' | 'Codex' | (string & {})
  category: SessionCategory
  kind: SessionKind | null
  title: string
  preview: string | null
  elapsedSeconds: number
  unread: boolean
  task: SessionTask | null
  repo: string
  parent?: { id: string; title: string | null } | null
  pullRequest: PullRequest | null
}

export type ChatItem =
  | { type: 'user'; text: string; from?: string }
  | { type: 'assistant'; text: string }
  | { type: 'tool_summary'; summary: string }
  | { type: 'question'; question: string; choices: string[] }
  | { type: 'permission'; tool: string; description: string }

export type SessionActions = {
  message: boolean
  answerQuestion: boolean
  permission?: { approve: boolean; reject: boolean }
  interrupt: boolean
  openInTerminal: boolean
}

export type SessionsResponse = {
  revision: number
  sessions: Session[]
}
