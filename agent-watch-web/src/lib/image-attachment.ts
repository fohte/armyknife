import { ResultAsync } from 'neverthrow'

const MAX_IMAGE_DIMENSION = 2048
const JPEG_QUALITY = 0.9
const IMAGE_EXTENSIONS = new Set([
  'avif',
  'gif',
  'heic',
  'heif',
  'jpeg',
  'jpg',
  'png',
  'webp',
])

type DecodedImage = {
  width: number
  height: number
  draw: (
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
  ) => void
}

type ImageAttachmentBoundary = {
  decode: (file: File) => Promise<DecodedImage>
  encodeJpeg: (
    image: DecodedImage,
    width: number,
    height: number,
    quality: number,
  ) => Promise<Blob | null>
}

const browserBoundary: ImageAttachmentBoundary = {
  decode: (file) =>
    new Promise((resolve, reject) => {
      const url = URL.createObjectURL(file)
      const image = new Image()
      const finish = (result: DecodedImage | Error) => {
        URL.revokeObjectURL(url)
        if (result instanceof Error) {
          reject(result)
        } else {
          resolve(result)
        }
      }

      image.onload = () => {
        if (image.naturalWidth === 0 || image.naturalHeight === 0) {
          finish(new Error('Image has no dimensions'))
          return
        }
        finish({
          width: image.naturalWidth,
          height: image.naturalHeight,
          draw: (context, width, height) => {
            context.drawImage(image, 0, 0, width, height)
          },
        })
      }
      image.onerror = () => {
        finish(new Error('Image could not be decoded'))
      }
      image.src = url
    }),
  encodeJpeg: (image, width, height, quality) => {
    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    const context = canvas.getContext('2d')
    if (context === null) return Promise.resolve(null)
    image.draw(context, width, height)
    return new Promise((resolve) => {
      canvas.toBlob(resolve, 'image/jpeg', quality)
    })
  },
}

function isImageFile(file: File) {
  if (file.type.toLowerCase().startsWith('image/')) return true
  const extension = file.name.split('.').at(-1)?.toLowerCase()
  return extension !== undefined && IMAGE_EXTENSIONS.has(extension)
}

function jpegFilename(name: string) {
  const stem = name.replace(/\.[^.]*$/, '') || 'Pasted image'
  return `${stem}.jpg`
}

export async function optimizeImageAttachment(
  file: File,
  boundary: ImageAttachmentBoundary = browserBoundary,
): Promise<File> {
  if (!isImageFile(file)) return file

  return ResultAsync.fromPromise(
    optimizeDecodedImage(file, boundary),
    (error) => (error instanceof Error ? error : new Error(String(error))),
  ).match(
    (optimizedFile) => optimizedFile,
    () => file,
  )
}

async function optimizeDecodedImage(
  file: File,
  boundary: ImageAttachmentBoundary,
) {
  const image = await boundary.decode(file)
  if (
    !Number.isFinite(image.width) ||
    !Number.isFinite(image.height) ||
    image.width <= 0 ||
    image.height <= 0
  ) {
    return file
  }

  const scale = Math.min(
    1,
    MAX_IMAGE_DIMENSION / Math.max(image.width, image.height),
  )
  const width = Math.max(1, Math.round(image.width * scale))
  const height = Math.max(1, Math.round(image.height * scale))
  const needsResize = width !== image.width || height !== image.height
  if (file.type.toLowerCase() === 'image/jpeg' && !needsResize) return file

  const blob = await boundary.encodeJpeg(image, width, height, JPEG_QUALITY)
  if (blob === null || blob.type !== 'image/jpeg') return file
  if (
    file.type.toLowerCase() === 'image/png' &&
    !needsResize &&
    file.size <= blob.size
  ) {
    return file
  }

  return new File([blob], jpegFilename(file.name), {
    type: 'image/jpeg',
    lastModified: file.lastModified,
  })
}
