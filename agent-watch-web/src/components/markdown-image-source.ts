import { Result } from 'neverthrow'

// react-markdown normalizes image URLs before they reach custom components.
function normalizeMarkdownUri(value: string): string {
  let normalized = ''
  for (let index = 0; index < value.length;) {
    const code = value.charCodeAt(index)
    const character = value[index] ?? ''
    const next = value[index + 1] ?? ''
    const afterNext = value[index + 2] ?? ''
    if (
      character === '%' &&
      /^[\da-z]$/i.test(next) &&
      /^[\da-z]$/i.test(afterNext)
    ) {
      normalized += value.slice(index, index + 3)
      index += 3
      continue
    }
    if (code < 128 && /[!#$&-;=?-Z_a-z~]/.test(character)) {
      normalized += character
      index += 1
      continue
    }

    let encodedCharacter = character
    let characterLength = 1
    if (code >= 0xd800 && code <= 0xdbff) {
      const nextCode = value.charCodeAt(index + 1)
      if (nextCode >= 0xdc00 && nextCode <= 0xdfff) {
        encodedCharacter = value.slice(index, index + 2)
        characterLength = 2
      } else {
        encodedCharacter = '\uFFFD'
      }
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      encodedCharacter = '\uFFFD'
    }
    normalized += encodeURIComponent(encodedCharacter)
    index += characterLength
  }
  return normalized
}

export function getMappedImageSource(
  src: string,
  imageSources: Readonly<Record<string, string | null>> | undefined,
): { path: string; source: string | null } | undefined {
  if (imageSources === undefined) return undefined
  const matches = Object.entries(imageSources).filter(
    ([path]) => path === src || normalizeMarkdownUri(path) === src,
  )
  if (matches.length === 0) return undefined
  if (matches.length > 1) return { path: src, source: null }
  const match = matches[0]
  if (match === undefined) return undefined
  return { path: match[0], source: match[1] }
}

export function displayMarkdownImagePath(src: string): string {
  return Result.fromThrowable(
    () => decodeURI(src),
    () => src,
  )().unwrapOr(src)
}

export function isUnmappedLocalImagePath(src: string): boolean {
  return (
    src.startsWith('/') &&
    src.slice(1).includes('/') &&
    !src.startsWith('//') &&
    !src.startsWith('/assets/') &&
    !src.startsWith('/api/images/') &&
    !src.startsWith('/api/sessions/')
  )
}
