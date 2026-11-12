type SessionCategory =
  'needs_action' | 'later' | 'in_progress' | 'done' | (string & {})

type SessionKind =
  | 'Question'
  | 'Approve'
  | 'Review'
  | 'Error'
  | 'Plan'
  | 'Proposal'
  | 'Todo'
  | 'Stopped'
  | (string & {})

export type SessionTask = {
  id: string
  number: number
  title: string
  priority: number | null
  isClosed?: boolean
}

export type PullRequest = {
  number: number
  state: 'Open' | 'Closed' | 'Merged' | (string & {})
}

type ParentSession = {
  id: string
  title: string | null
}

export type Session = {
  id: string
  engine?: 'claude' | 'codex'
  workType?: string | null
  category: SessionCategory
  isPaused: boolean
  activityStatus?: 'working' | 'waiting' | 'stopped'
  waitingReason?: string | null
  waitingForBackgroundTasks?: boolean
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
  hasSessionTree?: boolean
  ancestorSessionIds: string[]
  pullRequest: PullRequest | null
  critUrl?: string | null
  laterUntil?: string | null
  isToday?: boolean
  decisionCount?: number
  unreviewedDecisionCount?: number
}

export type SessionTreeNode = {
  id: string
  parentId: string | null
  category: SessionCategory
  isActive: boolean
  kind: SessionKind | null
  title: string
  preview: string | null
  elapsedSeconds: number
  task: SessionTask | null
  repo: string
  decisionCount: number
  pullRequest: PullRequest | null
}

export type SessionLinkTarget = Pick<Session, 'id' | 'repo'> & {
  task:
    | (Pick<SessionTask, 'id' | 'number' | 'title'> &
        Partial<Pick<SessionTask, 'priority'>>)
    | null
  linkedTaskCount?: number | undefined
} & {
  title?: Session['title'] | undefined
  engine?: Session['engine'] | undefined
  workType?: Session['workType'] | undefined
  kind?: Session['kind'] | undefined
  preview?: Session['preview'] | undefined
  elapsedSeconds?: Session['elapsedSeconds'] | undefined
}

export type ChatItem =
  | { type: 'user'; text: string; from?: string; timestamp?: string }
  | {
      type: 'queued_user'
      content: UserContentPart[]
      from?: string
      timestamp?: string
    }
  | {
      type: 'user_with_content'
      content: UserContentPart[]
      from?: string
      timestamp?: string
    }
  | {
      type: 'peer'
      direction: 'from' | 'to'
      session_id: string
      session_title?: string
      content: UserContentPart[]
      timestamp?: string
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
      kind: 'armyknife' | 'compact_boundary' | 'system' | 'hook'
      text: string
      failed: boolean
      timestamp?: string
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
      requestId?: string
      questions: QuestionPrompt[]
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
  | {
      type: 'todo'
      seq: number
      title: string
      detail: string | null
      completed: boolean
      comment: string | null
    }
  | {
      type: 'permission'
      requestId?: string
      tool: string
      description: string
    }
  | {
      type: 'decision'
      seq: number
      title: string
      why: string
      considered: string[]
      easyToChange: string
      review: 'ok' | 'change' | null
      createdAt: string
    }

type QuestionChoice = {
  label: string
  description?: string
}

export type QuestionPrompt = {
  header?: string
  question: string
  choices: QuestionChoice[]
  multi_select: boolean
  answer: string | null
}

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
  failed?: boolean
}

export type BackgroundTask = {
  id: string
  description: string
  elapsedSeconds: number
  kind: 'subagent' | 'background_task'
  agentType?: string
}

export type SkillSuggestion = {
  name: string
  description: string
}

export type ConversationResponse = {
  messages: ChatItem[]
  backgroundTasks: BackgroundTask[]
  skills?: SkillSuggestion[]
  start: number
  total: number
  decisions: Extract<ChatItem, { type: 'decision' }>[]
  hasUnansweredProposal: boolean
  activeQuestionIndex: number | null
}

export type SessionActions = {
  message: boolean
  answerQuestion: boolean
  permission?: { approve: boolean; reject: boolean }
  interrupt: boolean
  openInTerminal: boolean
}

export type OpenInTerminalStatus =
  | { state: 'sending' }
  | { state: 'success' }
  | { state: 'error'; message: string }
  | null

export type SessionsResponse = {
  revision: number
  workTypeNavLabels?: Record<string, string>
  sessions: Session[]
}
