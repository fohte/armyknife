import '#components/board-view.css'

import { Button } from '@fohte/ui/button'
import { useEffect, useRef, useState } from 'react'

import type { BoardProject } from '#board-model'
import { MobileNavigationActions } from '#components/mobile-navigation-actions'
import { SessionPausedBadge } from '#components/session-paused-badge'
import { SessionRow } from '#components/session-row'

type BoardViewProps = {
  projects: readonly BoardProject[]
  loaded: boolean
  error: string | null
  hideQuietProjects: boolean
  onHideQuietProjectsChange: (checked: boolean) => void
  onNewSession: () => void
  onSelectSession: (id: string) => void
  focusedProjectName?: string | null
  isMobile?: boolean
  onOpenNavigation?: () => void
  onOpenPalette?: () => void
}

const GROUPS = [
  {
    category: 'needs_action',
    title: 'Needs action',
    className: 'needs-action',
  },
  { category: 'in_progress', title: 'In progress', className: 'in-progress' },
  { category: 'done', title: 'Done', className: 'done' },
] as const

function ProjectCard({
  session,
  onSelectSession,
  isMobile,
}: {
  session: BoardProject['sessions'][number]
  onSelectSession: (id: string) => void
  isMobile: boolean
}) {
  return (
    <Button
      type="button"
      variant="plain"
      className={`session-row board-card block w-full cursor-pointer category-${session.category} hover:text-foreground`}
      data-kind={session.kind?.toLowerCase() ?? ''}
      aria-label={`Open in Inbox: ${session.title}`}
      onClick={() => {
        onSelectSession(session.id)
      }}
    >
      <SessionRow session={session} density="compact" isMobile={isMobile} />
    </Button>
  )
}

function ProjectColumn({
  project,
  onSelectSession,
  isMobile,
}: {
  project: BoardProject
  onSelectSession: (id: string) => void
  isMobile: boolean
}) {
  return (
    <section className="board-project" aria-label={project.name}>
      <header className="board-project-header">
        <h2>{project.name}</h2>
        {project.needsActionCount > 0 ? (
          <div className="board-project-need">
            {project.needsActionCount} need action
          </div>
        ) : (
          <div className="board-project-quiet">Nothing needed</div>
        )}
        {project.repos.length > 0 ? (
          <p className="board-project-repos">{project.repos.join(', ')}</p>
        ) : null}
      </header>
      {GROUPS.map(({ category, title, className }) => {
        const sessions = project.sessions.filter(
          (session) => session.category === category,
        )
        if (sessions.length === 0) {
          return null
        }

        if (category === 'done') {
          const doneSessionToSummarize =
            sessions.find((session) => session.isPaused) ?? sessions[0]

          return (
            <section
              key={category}
              className={`board-group ${className}`}
              aria-label={`${title} (${String(sessions.length)})`}
            >
              <p className="board-done-summary">
                <span className="board-done-label">✓ Done</span>
                <span aria-hidden="true"> · </span>
                {doneSessionToSummarize !== undefined ? (
                  <>
                    <SessionPausedBadge
                      isPaused={doneSessionToSummarize.isPaused}
                    />
                    {doneSessionToSummarize.title}
                    {sessions.length > 1
                      ? ` and ${String(sessions.length - 1)} more`
                      : ''}
                  </>
                ) : null}
              </p>
            </section>
          )
        }

        const visibleSessions = sessions.slice(0, 3)

        return (
          <section
            key={category}
            className={`board-group ${className}`}
            aria-label={`${title} (${String(sessions.length)})`}
          >
            <h3>{title}</h3>
            <div className="board-card-list">
              {visibleSessions.map((session) => (
                <ProjectCard
                  key={session.id}
                  session={session}
                  onSelectSession={onSelectSession}
                  isMobile={isMobile}
                />
              ))}
              {sessions.length > visibleSessions.length ? (
                <span className="board-more">
                  {sessions.length - visibleSessions.length} more
                </span>
              ) : null}
            </div>
          </section>
        )
      })}
    </section>
  )
}

export function BoardView({
  projects,
  loaded,
  error,
  hideQuietProjects,
  onHideQuietProjectsChange,
  onNewSession,
  onSelectSession,
  focusedProjectName = null,
  isMobile = false,
  onOpenNavigation = () => undefined,
  onOpenPalette = () => undefined,
}: BoardViewProps) {
  const projectsScrollRef = useRef<HTMLDivElement>(null)
  const [remainingProjects, setRemainingProjects] = useState(0)
  const [activeProjectIndex, setActiveProjectIndex] = useState(0)
  const quietCount = projects.filter((project) => project.quiet).length
  const focusedProject = projects.filter(
    (project) => project.name === focusedProjectName,
  )
  const visibleProjects =
    focusedProjectName !== null
      ? focusedProject
      : hideQuietProjects
        ? projects.filter((project) => !project.quiet)
        : projects

  useEffect(() => {
    const scroller = projectsScrollRef.current
    if (scroller === null) {
      return
    }

    const updateProjectIndicators = () => {
      const rightEdge = scroller.scrollLeft + scroller.clientWidth
      const columns = Array.from(
        scroller.querySelectorAll<HTMLElement>('.board-project'),
      )
      const offscreenProjects = columns.filter(
        (project) => project.offsetLeft + project.clientWidth > rightEdge,
      )
      setRemainingProjects(offscreenProjects.length)
      if (isMobile) {
        const scrollPaddingLeft =
          Number.parseFloat(getComputedStyle(scroller).scrollPaddingLeft) || 16
        const targetLeft = scroller.scrollLeft + scrollPaddingLeft
        const scrollerLeft = scroller.getBoundingClientRect().left
        const projectPositions = columns.map(
          (project) =>
            project.getBoundingClientRect().left -
            scrollerLeft +
            scroller.scrollLeft,
        )
        const currentIndex = projectPositions.reduce(
          (closestIndex, projectLeft, index) => {
            const closestLeft = projectPositions[closestIndex]
            if (closestLeft === undefined) {
              return index
            }
            return Math.abs(projectLeft - targetLeft) <
              Math.abs(closestLeft - targetLeft)
              ? index
              : closestIndex
          },
          0,
        )
        setActiveProjectIndex(currentIndex)
      }
    }

    updateProjectIndicators()
    scroller.addEventListener('scroll', updateProjectIndicators)
    const resizeObserver = new ResizeObserver(updateProjectIndicators)
    resizeObserver.observe(scroller)
    const projects = scroller.querySelector('.board-projects')
    if (projects !== null) {
      resizeObserver.observe(projects)
    }

    return () => {
      scroller.removeEventListener('scroll', updateProjectIndicators)
      resizeObserver.disconnect()
    }
  }, [isMobile, visibleProjects])

  return (
    <section
      className={`board-view${isMobile ? ' is-mobile' : ''}`}
      aria-labelledby="board-title"
    >
      <header className="board-header">
        <div className="board-header-main">
          <div className="board-header-title">
            {isMobile ? (
              <MobileNavigationActions
                onOpenNavigation={onOpenNavigation}
                onOpenPalette={onOpenPalette}
                onNewSession={onNewSession}
              />
            ) : null}
            <h1 id="board-title">Board</h1>
          </div>
          <label className="board-quiet-toggle">
            <input
              type="checkbox"
              checked={hideQuietProjects}
              onChange={(event) => {
                onHideQuietProjectsChange(event.currentTarget.checked)
              }}
            />
            <span>Hide quiet projects ({quietCount})</span>
          </label>
        </div>
        <p className="board-description">Projects that need you come first</p>
      </header>
      {error !== null ? (
        <p className="board-message is-error" role="status">
          {error}
        </p>
      ) : null}
      {!loaded ? (
        error === null ? (
          <p className="board-message" role="status">
            Loading projects…
          </p>
        ) : null
      ) : visibleProjects.length === 0 ? (
        <p className="board-message">No projects to show.</p>
      ) : (
        <>
          {isMobile ? (
            <div className="board-pagination-dots" aria-hidden="true">
              {visibleProjects.map((project, index) => (
                <span
                  key={project.name}
                  className="board-pagination-dot"
                  data-current={index === activeProjectIndex}
                />
              ))}
            </div>
          ) : null}
          <div className="board-projects-container">
            <div className="board-projects-scroll" ref={projectsScrollRef}>
              <div className="board-projects">
                {visibleProjects.map((project) => (
                  <ProjectColumn
                    key={project.name}
                    project={project}
                    onSelectSession={onSelectSession}
                    isMobile={isMobile}
                  />
                ))}
              </div>
            </div>
            {remainingProjects > 0 && !isMobile ? (
              <div className="board-overflow-hint" aria-hidden="true">
                {remainingProjects} more →
              </div>
            ) : null}
          </div>
        </>
      )}
    </section>
  )
}
