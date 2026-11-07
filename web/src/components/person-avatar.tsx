import { useEffect, useRef, useState } from 'react'
import type { RefObject } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery } from '@tanstack/react-query'
import { Avatar } from '@astryxdesign/core/Avatar'
import type { AvatarProps } from '@astryxdesign/core/Avatar'
import { HStack } from '@astryxdesign/core/HStack'

import { initialsOf, pictureQuery } from '@/lib/contacts'

const styles = stylex.create({
  box: { flexShrink: 0 },
})

interface PersonAvatarProps {
  name: string
  /** A saved contact's or a group's id; without one only initials show. */
  pictureID: string | null
  size?: AvatarProps['size']
  shape?: AvatarProps['shape']
}

/**
 * Initials until the WhatsApp picture has loaded, then the picture. It's asked for only once the
 * avatar is near the screen, so a long list doesn't send the server for every face at once.
 */
export function PersonAvatar({
  name,
  pictureID,
  size = 'md',
  shape = 'circle',
}: PersonAvatarProps) {
  const ref = useRef<HTMLSpanElement>(null)
  const isNear = useHasBeenNear(ref)
  const picture = useQuery({
    ...pictureQuery(pictureID ?? ''),
    enabled: Boolean(pictureID) && isNear,
  })
  return (
    <HStack as="span" ref={ref} xstyle={styles.box}>
      <Avatar
        name={initialsOf(name)}
        src={picture.data ?? undefined}
        size={size}
        shape={shape}
        tooltip={false}
      />
    </HStack>
  )
}

function useHasBeenNear(ref: RefObject<HTMLElement | null>) {
  const [hasBeenNear, setHasBeenNear] = useState(false)
  useEffect(() => {
    const element = ref.current
    if (!element || hasBeenNear) return
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry?.isIntersecting) setHasBeenNear(true)
      },
      { rootMargin: '200px' },
    )
    observer.observe(element)
    return () => observer.disconnect()
  }, [ref, hasBeenNear])
  return hasBeenNear
}
