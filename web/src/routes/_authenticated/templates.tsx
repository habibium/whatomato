import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { useAnnounce, useMediaQuery } from '@astryxdesign/core/hooks'
import { MoreMenu } from '@astryxdesign/core/MoreMenu'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { Table, pixel, proportional } from '@astryxdesign/core/Table'
import { Text } from '@astryxdesign/core/Text'
import { Timestamp } from '@astryxdesign/core/Timestamp'
import { VisuallyHidden } from '@astryxdesign/core/VisuallyHidden'
import { VStack } from '@astryxdesign/core/VStack'

import type { TableColumn } from '@astryxdesign/core/Table'

import { api } from '@/api/client'
import { AppFrame } from '@/components/app-frame'
import { ConfirmDeleteDialog } from '@/components/confirm-dialog'
import { ListEmpty } from '@/components/list-empty'
import { PageHeading } from '@/components/page-heading'
import { TemplateFormDialog } from '@/components/template-form-dialog'
import { messagesQuery, templatesQuery } from '@/lib/scheduler'
import type { Template } from '@/lib/scheduler'
import { useSessionExpiry } from '@/lib/use-session-expiry'
import { connectionProblem, toWriteProblem } from '@/lib/write-problem'
import type { WriteProblem } from '@/lib/write-problem'

export const Route = createFileRoute('/_authenticated/templates')({
  component: TemplatesPage,
})

type OpenDialog =
  | { kind: 'write'; template: Template | null }
  | { kind: 'delete'; template: Template }

function TemplatesPage() {
  const queryClient = useQueryClient()
  const announce = useAnnounce()
  const query = useQuery(templatesQuery)
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
  const newTemplate = (
    <Button
      label="New template"
      variant="primary"
      size="lg"
      clickAction={() => setDialog({ kind: 'write', template: null })}
    />
  )

  return (
    <AppFrame>
      <VStack gap={8} paddingBlock={10}>
        <PageHeading
          title="Message templates."
          lede="Text you send often. Apply one when you write a message, then change what you need."
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
            <TemplateTable
              templates={templates}
              onEdit={(template) => setDialog({ kind: 'write', template })}
              onDelete={(template) => setDialog({ kind: 'delete', template })}
            />
          ) : (
            <ListEmpty
              title="No templates yet"
              description="Save the text you send often, and it’s one click away when you write a message."
              actions={newTemplate}
            />
          )
        ) : query.isPending ? (
          <VStack gap={3} aria-busy="true">
            {[0, 1, 2].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={44} />
            ))}
          </VStack>
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

interface TemplateTableProps {
  templates: Template[]
  onEdit: (template: Template) => void
  onDelete: (template: Template) => void
}

function TemplateTable({ templates, onEdit, onDelete }: TemplateTableProps) {
  const isNarrow = useMediaQuery('(max-width: 639px)')

  const menu: TableColumn<Template> = {
    key: 'actions',
    header: <VisuallyHidden>Actions</VisuallyHidden>,
    width: pixel(52),
    renderCell: (template) => (
      <MoreMenu
        label={`Actions for ${template.name}`}
        alignment="end"
        size="sm"
        items={[
          { label: 'Edit', onClick: () => onEdit(template) },
          {
            label: 'Delete',
            variant: 'destructive',
            onClick: () => onDelete(template),
          },
        ]}
      />
    ),
  }

  const columns: TableColumn<Template>[] = isNarrow
    ? [
        {
          key: 'template',
          header: 'Template',
          width: proportional(1),
          renderCell: (template) => (
            <VStack gap={1}>
              <Text weight="medium" maxLines={1}>
                {template.name}
              </Text>
              <Text color="secondary" maxLines={2}>
                {template.body}
              </Text>
            </VStack>
          ),
        },
        menu,
      ]
    : [
        {
          key: 'name',
          header: 'Name',
          width: proportional(2),
          renderCell: (template) => (
            <Text weight="medium" maxLines={1}>
              {template.name}
            </Text>
          ),
        },
        {
          key: 'body',
          header: 'Message',
          width: proportional(5),
          renderCell: (template) => (
            <Text color="secondary" maxLines={2}>
              {template.body}
            </Text>
          ),
        },
        {
          key: 'updated',
          header: 'Updated',
          width: pixel(128),
          renderCell: (template) => (
            <Timestamp
              value={template.updated_at}
              format="date"
              type="body"
              color="secondary"
            />
          ),
        },
        menu,
      ]

  return (
    <Table
      data={templates}
      columns={columns}
      idKey="id"
      verticalAlign={isNarrow ? 'top' : 'middle'}
    />
  )
}
