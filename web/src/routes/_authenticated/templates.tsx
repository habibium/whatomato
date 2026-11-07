import { useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { Card } from '@astryxdesign/core/Card'
import { Grid } from '@astryxdesign/core/Grid'
import { HStack } from '@astryxdesign/core/HStack'
import { useAnnounce } from '@astryxdesign/core/hooks'
import { MoreMenu } from '@astryxdesign/core/MoreMenu'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { Heading, Text } from '@astryxdesign/core/Text'
import { useToast } from '@astryxdesign/core/Toast'
import { VStack } from '@astryxdesign/core/VStack'
import { typeScaleVars } from '@astryxdesign/core/theme/tokens.stylex'

import { api } from '@/api/client'
import { AppFrame } from '@/components/app-frame'
import { ConfirmDeleteDialog } from '@/components/confirm-dialog'
import { ListEmpty } from '@/components/list-empty'
import { MessageBubble } from '@/components/message-bubble'
import { MessageFormDialog } from '@/components/message-form-dialog'
import { PageHeading } from '@/components/page-heading'
import { TemplateFormDialog } from '@/components/template-form-dialog'
import { formatDay } from '@/lib/format'
import { messagesQuery, templatesQuery } from '@/lib/scheduler'
import type { Template } from '@/lib/scheduler'
import { useSessionExpiry } from '@/lib/use-session-expiry'
import { connectionProblem, toWriteProblem } from '@/lib/write-problem'
import type { WriteProblem } from '@/lib/write-problem'

export const Route = createFileRoute('/_authenticated/templates')({
  component: TemplatesPage,
})

const styles = stylex.create({
  card: { height: '100%' },
  title: {
    fontSize: typeScaleVars['--text-heading-3-size'],
    lineHeight: typeScaleVars['--text-heading-3-leading'],
    minWidth: 0,
  },
  // The action sits on the card's foot, so cards in one row end on one line
  foot: { marginTop: 'auto' },
})

type OpenDialog =
  | { kind: 'write'; template: Template | null }
  | { kind: 'delete'; template: Template }
  | { kind: 'use'; template: Template }

function TemplatesPage() {
  const queryClient = useQueryClient()
  const announce = useAnnounce()
  const toast = useToast()
  const query = useQuery(templatesQuery)
  const messages = useQuery(messagesQuery)
  const isSignedOut = useSessionExpiry(query.error, '/templates')
  const [dialog, setDialog] = useState<OpenDialog | null>(null)
  const [problem, setProblem] = useState<WriteProblem | null>(null)

  const closeDialog = (isOpen: boolean) => {
    if (!isOpen) setDialog(null)
  }

  const remove = async (template: Template) => {
    const failed = "Couldn't delete this template."
    setProblem(null)
    try {
      const result = await api.DELETE('/api/templates/{id}', {
        params: { path: { id: template.id } },
      })
      // 404: someone already deleted it, which is what was asked for
      if (result.response.ok || result.response.status === 404) {
        announce('Template deleted.')
        toast({ body: 'Template deleted.', type: 'info' })
      } else {
        setProblem(toWriteProblem(result, failed))
      }
    } catch {
      setProblem(connectionProblem(failed))
    }
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: templatesQuery.queryKey }),
      // Messages that used it lose their link to it
      queryClient.invalidateQueries({ queryKey: messagesQuery.queryKey }),
    ])
    setDialog(null)
  }

  const templates = query.data
  const uses = new Map<string, number>()
  for (const message of messages.data ?? []) {
    if (message.template_id) {
      uses.set(message.template_id, (uses.get(message.template_id) ?? 0) + 1)
    }
  }
  const newTemplate = (
    <Button
      label="New template"
      variant="primary"
      clickAction={() => setDialog({ kind: 'write', template: null })}
    />
  )

  return (
    <AppFrame>
      <VStack gap={6}>
        <PageHeading
          title="Templates"
          lede="Text you send often. Start a message from one, then change what you need."
          actions={templates?.length === 0 ? undefined : newTemplate}
        />

        {problem ? (
          <Banner
            status="error"
            title={problem.title}
            description={problem.description}
            isDismissable
            onDismiss={() => setProblem(null)}
          />
        ) : null}

        {templates ? (
          templates.length > 0 ? (
            <Grid columns={{ minWidth: 280, repeat: 'fill' }} gap={4}>
              {[...templates]
                .sort((a, b) => a.name.localeCompare(b.name))
                .map((template) => (
                  <TemplateCard
                    key={template.id}
                    template={template}
                    uses={messages.data ? (uses.get(template.id) ?? 0) : null}
                    onUse={() => setDialog({ kind: 'use', template })}
                    onEdit={() => setDialog({ kind: 'write', template })}
                    onDelete={() => setDialog({ kind: 'delete', template })}
                  />
                ))}
            </Grid>
          ) : (
            <ListEmpty
              title="No templates yet"
              description="Save the text you send often, and it’s one click away when you write a message."
              actions={newTemplate}
            />
          )
        ) : query.isPending ? (
          <Grid
            columns={{ minWidth: 280, repeat: 'fill' }}
            gap={4}
            aria-busy="true"
          >
            {[0, 1, 2].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={188} />
            ))}
          </Grid>
        ) : isSignedOut ? null : (
          <Banner
            status="error"
            title="Couldn't load your templates."
            description="Check your connection and try again."
            endContent={
              <Button
                label="Try again"
                size="sm"
                clickAction={async () => {
                  await query.refetch()
                }}
              />
            }
          />
        )}
      </VStack>

      <TemplateFormDialog
        isOpen={dialog?.kind === 'write'}
        onOpenChange={closeDialog}
        template={dialog?.kind === 'write' ? dialog.template : null}
      />
      <MessageFormDialog
        isOpen={dialog?.kind === 'use'}
        onOpenChange={closeDialog}
        message={null}
        template={dialog?.kind === 'use' ? dialog.template : null}
      />
      <ConfirmDeleteDialog
        isOpen={dialog?.kind === 'delete'}
        onOpenChange={closeDialog}
        title="Delete this template?"
        description="Messages already using it keep their text."
        onConfirm={async () => {
          if (dialog?.kind === 'delete') await remove(dialog.template)
        }}
      />
    </AppFrame>
  )
}

interface TemplateCardProps {
  template: Template
  /** `null` while messages are still loading. */
  uses: number | null
  onUse: () => void
  onEdit: () => void
  onDelete: () => void
}

function TemplateCard({
  template,
  uses,
  onUse,
  onEdit,
  onDelete,
}: TemplateCardProps) {
  const day = formatDay(template.updated_at)
  const edited = /^(Today|Yesterday)$/.test(day)
    ? day.toLowerCase()
    : `on ${day}`
  const usage =
    uses === null
      ? null
      : uses === 0
        ? 'Not used yet'
        : `Used by ${uses} ${uses === 1 ? 'message' : 'messages'}`
  return (
    <Card padding={4} xstyle={styles.card}>
      <VStack gap={3} xstyle={styles.card}>
        <HStack gap={2} justify="between" vAlign="center">
          <Heading level={2} xstyle={styles.title}>
            {template.name}
          </Heading>
          <MoreMenu
            label={`Actions for ${template.name}`}
            alignment="end"
            size="sm"
            items={[
              { label: 'Edit', onClick: onEdit },
              { label: 'Delete', variant: 'destructive', onClick: onDelete },
            ]}
          />
        </HStack>
        <MessageBubble body={template.body} maxLines={4} />
        <HStack gap={3} justify="between" vAlign="center" xstyle={styles.foot}>
          <Text type="supporting">
            {usage ? `${usage} · ` : null}Edited {edited}
          </Text>
          <Button label="Use" size="sm" clickAction={onUse} />
        </HStack>
      </VStack>
    </Card>
  )
}
