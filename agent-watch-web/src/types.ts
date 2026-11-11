type SessionCategory =
  'needs_action' | 'later' | 'in_progress' | 'done' | (string & {})

type SessionKind =
  | 'Question'
  | 'Approve'
  | 'Review'
  | 'Error'
  | 'Plan'
  | 'Proposal'
  | 'Stopped'
  | (string & {})

export type SessionTask = {
  id: string
  number: number
  title: string
  priority: number | null
  isClosed?: boolean
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
  engine?: 'Claude' | 'Codex' | (string & {})
  workType?: string | null
  category: SessionCategory
  isPaused: boolean
  isCompacting?: boolean
  categoryWhenNotLater?: SessionCategory
  kind: SessionKind | null
  title: string
  preview: string | null
  elapsedSeconds: number
  unread: boolean
  task: SessionTask | null
  tasks?: SessionTask[]
  repo: string
  branch?: string | null
  canOpenInTerminal?: boolean
  project: string
  parent: ParentSession | null
  ancestorSessionIds: string[]
  pullRequest: PullRequest | null
  critUrl?: string | null
  laterUntil?: string | null
  isToday?: boolean
}

export type SessionLinkTarget = Pick<
  Session,
  'id' | 'category' | 'isPaused' | 'task' | 'repo'
> &
  Partial<Pick<Session, 'title'>>

export type ChatItem =
  | { type: 'user'; text: string; from?: string }
  | { type: 'queued_user'; content: UserContentPart[]; from?: string }
  | {
      type: 'user_with_content'
      content: UserContentPart[]
      from?: string
    }
  | {
      type: 'peer'
      direction: 'from' | 'to'
      session_id: string
      session_title?: string
      content: UserContentPart[]
    }
  | {
      type: 'subagent_report'
      agent_id: string
      agent_name: string
      description?: string
      text: string
    }
  | {
      type: 'event'
      kind: 'armyknife' | 'system' | 'hook'
      text: string
      failed: boolean
    }
  | { type: 'assistant'; text: string }
  | {
      type: 'tool_summary'
      summary: string
      operations?: ToolOperation[]
    }
  | { type: 'tool_in_progress'; tool: string; target?: string }
  | {
      type: 'question'
      question: string
      choices: string[]
      answer: string | null
    }
  | {
      type: 'proposal'
      seq: number
      title: string
      summary: string | null
      options: ProposalOption[]
      answer: ProposalAnswer | null
      resolved: boolean
    }
  | { type: 'permission'; tool: string; description: string }

export type ProposalOption = {
  label: string
  description: string | null
  pros: string[]
  cons: string[]
  recommended: boolean
}

export type ProposalAnswer = {
  choice: number | null
  comment: string | null
}

export type UserContentPart =
  | { type: 'text'; text: string }
  | { type: 'image'; source: string; available: boolean }
  | { type: 'file'; name: string }

export type ToolOperation = {
  tool: string
  target?: string
}

export type BackgroundTask = {
  id: string
  description: string
  elapsedSeconds: number
}

export type ConversationResponse = {
  messages: ChatItem[]
  backgroundTasks: BackgroundTask[]
}

export type SessionActions = {
  message: boolean
  answerQuestion: boolean
  permission?: { approve: boolean; reject: boolean }
  interrupt: boolean
  openInTerminal: boolean
}

export type SessionsResponse = {
  revision: number
  workTypeNavLabels?: Record<string, string>
  sessions: Session[]
}
