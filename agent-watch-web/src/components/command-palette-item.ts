import type { PaletteItem } from '#command-palette-model'
import { formatLinkedTaskCount } from '#session-task-count'
import type { Session } from '#types'

export function itemTitle(item: PaletteItem) {
  switch (item.type) {
    case 'session':
      return item.session.title
    case 'task':
      return item.task.title
    case 'project':
      return item.name
    case 'command':
      return item.label
  }
}

export function itemResultTitle(item: PaletteItem) {
  if (item.type === 'task') {
    return `#${String(item.task.number)} ${item.task.title}`
  }
  return itemTitle(item)
}

export function itemSubtitle(item: PaletteItem) {
  switch (item.type) {
    case 'session': {
      const linkedTaskCount = formatLinkedTaskCount(item.session.tasks)
      const task = item.session.task
        ? `P${String(item.session.task.priority ?? '—')} #${String(item.session.task.number)} ${item.session.task.title}${linkedTaskCount} · `
        : ''
      const branch = item.session.branch?.trim()
      const branchLabel =
        branch === undefined || branch.length === 0 ? '' : `${branch} · `
      return `${task}${branchLabel}${item.session.repo}`
    }
    case 'task':
      return aggregateSubtitle(item.sessions, item.task.priority)
    case 'project':
      return aggregateSubtitle(item.sessions)
    case 'command':
      return item.description
  }
}

function aggregateSubtitle(
  sessions: readonly Session[],
  priority?: number | null,
) {
  const needsActionCount = sessions.filter(
    (session) => session.category === 'needs_action',
  ).length
  const repos = Array.from(new Set(sessions.map((session) => session.repo)))
  const priorityLabel =
    priority === undefined ? '' : `P${String(priority ?? '—')} · `
  return `${priorityLabel}${String(sessions.length)} sessions · ${String(needsActionCount)} need action · ${repos.join(', ')}`
}

export function itemDetail(item: PaletteItem) {
  switch (item.type) {
    case 'session': {
      if (
        item.session.category === 'later' ||
        item.session.category === 'done'
      ) {
        return ''
      }
      const elapsedSeconds = Math.floor(item.session.elapsedSeconds)
      return elapsedSeconds < 60
        ? `${String(elapsedSeconds)}s`
        : `${String(Math.floor(elapsedSeconds / 60))}m`
    }
    case 'task':
      return 'Show in Board'
    case 'project':
      return 'Show in Board'
    case 'command':
      return ''
  }
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

export function itemColor(item: PaletteItem) {
  if (item.type === 'session') {
    if (item.session.kind === 'Error') return 'error'
    return item.session.category.replace('_', '-')
  }
  if (item.type === 'command') return 'command'
  return item.type
}

export function itemIcon(item: PaletteItem) {
  if (item.type === 'task') return '#'
  if (item.type === 'project') return '▤'
  if (item.type === 'command') return '›'
  if (item.session.kind === 'Error') return '×'

  switch (item.session.category) {
    case 'needs_action':
      return '◆'
    case 'later':
      return '◷'
    case 'in_progress':
      return '●'
    case 'done':
      return '✓'
    default:
      return '•'
  }
}
