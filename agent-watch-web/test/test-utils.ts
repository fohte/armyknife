import { act } from 'react'

export function setTextareaValue(
  textarea: HTMLTextAreaElement,
  value: string,
  cursor = value.length,
) {
  act(() => {
    const valueSetter = Reflect.getOwnPropertyDescriptor(
      textarea.constructor.prototype,
      'value',
    )?.set
    if (valueSetter !== undefined) Reflect.apply(valueSetter, textarea, [value])
    textarea.setSelectionRange(cursor, cursor)
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
