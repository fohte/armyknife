import type { SessionTreeNode } from '#types'

export type VisibleSessionTreeRow = {
  session: SessionTreeNode
  depth: number
  hasChildren: boolean
  isExpanded: boolean
  needsActionDescendantCount: number
}

function childrenByParent(sessions: readonly SessionTreeNode[]) {
  const sessionIds = new Set(sessions.map((session) => session.id))
  const children = new Map<string | null, SessionTreeNode[]>()
  for (const session of sessions) {
    const parentId =
      session.parentId !== null && sessionIds.has(session.parentId)
        ? session.parentId
        : null
    children.set(parentId, [...(children.get(parentId) ?? []), session])
  }
  return children
}

export function defaultExpandedSessionIds(
  sessions: readonly SessionTreeNode[],
  selectedId: string,
): Set<string> {
  const children = childrenByParent(sessions)
  const byId = new Map(sessions.map((session) => [session.id, session]))
  const expanded = new Set<string>()
  expanded.add(selectedId)
  let current = byId.get(selectedId)
  while (current !== undefined) {
    if (current.parentId !== null) expanded.add(current.parentId)
    current = current.parentId === null ? undefined : byId.get(current.parentId)
  }
  const openDescendants = (id: string) => {
    for (const child of children.get(id) ?? []) {
      expanded.add(id)
      openDescendants(child.id)
    }
  }
  openDescendants(selectedId)
  return expanded
}

export function visibleSessionTreeRows(
  sessions: readonly SessionTreeNode[],
  expandedIds: ReadonlySet<string>,
): VisibleSessionTreeRow[] {
  const children = childrenByParent(sessions)
  const needsActionCount = (session: SessionTreeNode): number =>
    (children.get(session.id) ?? []).reduce(
      (count, child) =>
        count +
        (child.category === 'needs_action' ? 1 : 0) +
        needsActionCount(child),
      0,
    )
  const rows: VisibleSessionTreeRow[] = []
  const visit = (session: SessionTreeNode, depth: number) => {
    const childSessions = children.get(session.id) ?? []
    rows.push({
      session,
      depth,
      hasChildren: childSessions.length > 0,
      isExpanded: expandedIds.has(session.id),
      needsActionDescendantCount: needsActionCount(session),
    })
    if (expandedIds.has(session.id)) {
      for (const child of childSessions) visit(child, depth + 1)
    }
  }
  for (const root of children.get(null) ?? []) visit(root, 0)
  return rows
}

export function needsActionDescendantCount(
  sessions: readonly SessionTreeNode[],
  selectedId: string,
): number {
  const rows = visibleSessionTreeRows(
    sessions,
    new Set(sessions.map(({ id }) => id)),
  )
  return (
    rows.find((row) => row.session.id === selectedId)
      ?.needsActionDescendantCount ?? 0
  )
}
