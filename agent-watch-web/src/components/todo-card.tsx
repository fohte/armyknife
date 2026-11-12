import '#components/todo-card.css'

import { Button } from '@fohte/ui/button'
import { useEffect, useId, useState } from 'react'

type TodoCardProps = {
  title: string
  detail: string | null
  completed: boolean
  defaultComment?: string
  onComplete: (comment: string | null) => Promise<boolean>
}

export function TodoCard({
  title,
  detail,
  completed,
  defaultComment = '',
  onComplete,
}: TodoCardProps) {
  const commentId = useId()
  const [comment, setComment] = useState(defaultComment)
  const [completedLocally, setCompletedLocally] = useState(false)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const isCompleted = completed || completedLocally

  useEffect(() => {
    if (completed) setComment(defaultComment)
  }, [completed, defaultComment])

  const completeTodo = () => {
    if (isCompleted || isSubmitting) return
    const normalizedComment = comment.trim()
    setIsSubmitting(true)
    void onComplete(normalizedComment === '' ? null : normalizedComment)
      .then((succeeded) => {
        if (succeeded) setCompletedLocally(true)
      })
      .finally(() => {
        setIsSubmitting(false)
      })
  }

  if (isCompleted) {
    const completionComment = completed ? defaultComment : comment.trim()

    return (
      <article
        className="chat-todo-card"
        aria-label="Completed user task"
        data-completed="true"
      >
        <h2>To-do · done</h2>
        <h3>{title}</h3>
        {completionComment !== '' ? (
          <p className="chat-todo-completed-comment">{completionComment}</p>
        ) : null}
      </article>
    )
  }

  return (
    <article
      className="chat-todo-card"
      aria-label="User task"
      aria-busy={isSubmitting}
    >
      <h2>To-do · pending</h2>
      <h3>{title}</h3>
      {detail !== null ? <p className="chat-todo-detail">{detail}</p> : null}
      <label className="chat-todo-comment-label" htmlFor={commentId}>
        Comment (optional)
      </label>
      <textarea
        id={commentId}
        className="chat-todo-comment"
        rows={2}
        value={comment}
        disabled={isSubmitting}
        onChange={(event) => {
          setComment(event.currentTarget.value)
        }}
      />
      <footer className="chat-todo-footer">
        <Button
          type="button"
          variant="plain"
          className="aw-button aw-button--primary"
          disabled={isSubmitting}
          onClick={completeTodo}
        >
          Mark done
        </Button>
      </footer>
    </article>
  )
}
