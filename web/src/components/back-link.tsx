import { HStack } from '@astryxdesign/core/HStack'
import { Link } from '@astryxdesign/core/Link'
import { ArrowLeft } from 'lucide-react'

/** The way up from a page below another, as the contact page has it. */
export function BackLink({ href, label }: { href: string; label: string }) {
  return (
    <HStack>
      <Link href={href} hasUnderline>
        <HStack gap={1} vAlign="center">
          <ArrowLeft size={16} aria-hidden="true" />
          {label}
        </HStack>
      </Link>
    </HStack>
  )
}
