import { HStack } from '@astryxdesign/core/HStack'
import { Text } from '@astryxdesign/core/Text'
import { Bot } from 'lucide-react'

/** Marks what an AI agent made, in place of a column that would be empty for most rows. */
export function ByAgent({ name }: { name: string }) {
  return (
    <HStack gap={1} vAlign="center">
      <Bot size={14} aria-hidden="true" />
      <Text type="supporting">By {name}</Text>
    </HStack>
  )
}
