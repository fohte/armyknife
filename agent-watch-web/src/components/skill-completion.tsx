import '#components/skill-completion.css'

import type { ChangeEvent, KeyboardEvent, Ref, SyntheticEvent } from 'react'
import { useId, useLayoutEffect, useRef, useState } from 'react'

import type { SkillSuggestion } from '#types'

type Engine = 'claude' | 'codex' | undefined

type SkillCompletionProps = {
  draft: string
  engine: Engine
  skills: readonly SkillSuggestion[]
  onDraftChange: (draft: string) => void
  onKeyDown?: (event: KeyboardEvent<HTMLTextAreaElement>) => void
}

type CompletionToken = {
  trigger: string
  query: string
  start: number
  end: number
}

export function useSkillCompletion({
  draft,
  engine,
  skills,
  onDraftChange,
  onKeyDown,
}: SkillCompletionProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const suggestionsRef = useRef<HTMLUListElement>(null)
  const pendingCursorPosition = useRef<number | null>(null)
  const suggestionsId = useId()
  const [activeSkillIndex, setActiveSkillIndex] = useState(0)
  const [dismissedCompletion, setDismissedCompletion] = useState<string | null>(
    null,
  )
  const [caret, setCaret] = useState<{
    draft: string
    position: number
  } | null>(null)

  const cursorPosition = caret?.draft === draft ? caret.position : draft.length
  const completion = findSkillCompletion(draft, engine, cursorPosition)
  const matchingSkills =
    completion === null
      ? []
      : skills.filter((skill) =>
          skill.name
            .toLocaleLowerCase()
            .startsWith(completion.query.toLocaleLowerCase()),
        )
  const completionKey =
    completion === null
      ? null
      : `${engine ?? ''}:${String(completion.start)}:${completion.trigger}:${completion.query}`
  const isOpen =
    completion !== null &&
    matchingSkills.length > 0 &&
    completionKey !== dismissedCompletion
  const selectedSkillIndex = Math.min(
    activeSkillIndex,
    Math.max(0, matchingSkills.length - 1),
  )

  useLayoutEffect(() => {
    if (pendingCursorPosition.current === null) return
    const cursor = pendingCursorPosition.current
    pendingCursorPosition.current = null
    textareaRef.current?.focus()
    textareaRef.current?.setSelectionRange(cursor, cursor)
  }, [draft])

  const selectSkill = (skill: SkillSuggestion) => {
    if (completion === null || completionKey === null) return
    const afterToken = draft.slice(completion.end)
    const separator = /^\s/.test(afterToken) ? '' : ' '
    const insertion = `${completion.trigger}${skill.name}${separator}`
    const nextDraft = draft.slice(0, completion.start) + insertion + afterToken
    const nextCursor = completion.start + insertion.length
    if (nextDraft === draft) {
      pendingCursorPosition.current = null
      textareaRef.current?.focus()
      textareaRef.current?.setSelectionRange(nextCursor, nextCursor)
    } else {
      pendingCursorPosition.current = nextCursor
    }
    setCaret({ draft: nextDraft, position: nextCursor })
    onDraftChange(nextDraft)
    setDismissedCompletion(
      `${engine ?? ''}:${String(completion.start)}:${completion.trigger}:${skill.name}`,
    )
  }

  const handleChange = (event: ChangeEvent<HTMLTextAreaElement>) => {
    const nextDraft = event.currentTarget.value
    setCaret({ draft: nextDraft, position: event.currentTarget.selectionStart })
    setDismissedCompletion(null)
    setActiveSkillIndex(0)
    onDraftChange(nextDraft)
  }

  const handleSelect = (event: SyntheticEvent<HTMLTextAreaElement>) => {
    setCaret({
      draft: event.currentTarget.value,
      position: event.currentTarget.selectionStart,
    })
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (
      isOpen &&
      !event.nativeEvent.isComposing &&
      Reflect.get(event.nativeEvent, 'keyCode') !== 229
    ) {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault()
        const direction = event.key === 'ArrowDown' ? 1 : -1
        setActiveSkillIndex(
          (index) =>
            (index + direction + matchingSkills.length) % matchingSkills.length,
        )
        return
      }
      if (event.key === 'Enter' || event.key === 'Tab') {
        const skill = matchingSkills[selectedSkillIndex]
        if (skill !== undefined) {
          event.preventDefault()
          selectSkill(skill)
          return
        }
      }
      if (event.key === 'Escape' && completionKey !== null) {
        event.preventDefault()
        event.stopPropagation()
        dismiss()
        return
      }
    }
    onKeyDown?.(event)
  }

  const dismiss = () => {
    if (completionKey !== null) setDismissedCompletion(completionKey)
  }

  return {
    textareaRef,
    suggestionsRef,
    handleChange,
    handleSelect,
    handleKeyDown,
    completion,
    matchingSkills,
    selectedSkillIndex,
    suggestionsId,
    isOpen,
    selectSkill,
    dismiss,
  }
}

export function SkillSuggestionList({
  ref,
  id,
  completion,
  skills,
  selectedIndex,
  onSelect,
}: {
  ref?: Ref<HTMLUListElement>
  id: string
  completion: CompletionToken | null
  skills: readonly SkillSuggestion[]
  selectedIndex: number
  onSelect: (skill: SkillSuggestion) => void
}) {
  if (completion === null || skills.length === 0) return null

  return (
    <ul
      ref={ref}
      id={id}
      className="composer-skill-suggestions"
      role="listbox"
      aria-label="Available skills"
    >
      {skills.map((skill, index) => (
        <li key={skill.name} role="presentation">
          <button
            id={`${id}-skill-${String(index)}`}
            className="composer-skill-option"
            type="button"
            role="option"
            aria-selected={index === selectedIndex}
            onMouseDown={(event) => {
              event.preventDefault()
            }}
            onClick={() => {
              onSelect(skill)
            }}
          >
            <span>
              {completion.trigger}
              {skill.name}
            </span>
            <small>{skill.description}</small>
          </button>
        </li>
      ))}
    </ul>
  )
}

export function skillCompletionTextareaAttributes({
  isOpen,
  id,
  selectedIndex,
}: {
  isOpen: boolean
  id: string
  selectedIndex: number
}) {
  return {
    'aria-autocomplete': isOpen ? ('list' as const) : undefined,
    'aria-expanded': isOpen,
    'aria-controls': isOpen ? id : undefined,
    'aria-activedescendant': isOpen
      ? `${id}-skill-${String(selectedIndex)}`
      : undefined,
  }
}

function findSkillCompletion(
  draft: string,
  engine: Engine,
  cursor: number,
): CompletionToken | null {
  if (engine !== 'claude' && engine !== 'codex') return null
  const beforeCursor = draft.slice(0, cursor)
  const match = /(?:^|\s)([/$])([^\s]*)$/.exec(beforeCursor)
  if (match === null || match[1] === undefined || match[2] === undefined) {
    return null
  }
  const trigger = match[1]
  if (
    (engine === 'claude' && trigger !== '/') ||
    (engine === 'codex' && trigger !== '$')
  ) {
    return null
  }
  const start = cursor - match[0].length + match[0].lastIndexOf(trigger)
  let end = cursor
  while (end < draft.length && !/\s/.test(draft[end] ?? '')) end += 1
  return { trigger, query: match[2], start, end }
}
