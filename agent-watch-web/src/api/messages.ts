export type SessionMessage =
  | { type: 'user'; text: string; from?: string }
  | { type: 'assistant'; text: string }
  | { type: 'tool_summary'; summary: string }
  | { type: 'question'; question: string; choices: string[] }
  | { type: 'permission'; tool: string; description: string }
