export function unwrapTaggedMessage(value: string): string {
  const match = value
    .trim()
    .match(/^<([A-Za-z_][A-Za-z0-9._:-]*)>\r?\n?([\s\S]*?)\r?\n?<\/\1>$/)
  const tag = match?.[1]
  const body = match?.[2]
  return tag === undefined ||
    body === undefined ||
    /^(details|summary)$/i.test(tag)
    ? value
    : `## ${tag}\n\n${body.trim()}`
}
