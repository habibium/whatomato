import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { useForm } from '@tanstack/react-form'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog'
import { FormLayout } from '@astryxdesign/core/FormLayout'
import { HStack } from '@astryxdesign/core/HStack'
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout'
import { TextInput } from '@astryxdesign/core/TextInput'
import { VStack } from '@astryxdesign/core/VStack'
import { z } from 'zod'

import { api } from '@/api/client'
import { DialogForm } from '@/components/dialog-form'
import { MessageBodyField } from '@/components/message-body-field'
import { toFieldErrors, toFieldStatus } from '@/lib/form-errors'
import { BODY_MAX, bodySchema, templatesQuery } from '@/lib/scheduler'
import type { Template } from '@/lib/scheduler'
import { connectionProblem, toWriteProblem } from '@/lib/write-problem'
import type { WriteProblem } from '@/lib/write-problem'

const NAME_MAX = 100

const schema = z.object({
  name: z
    .string()
    .trim()
    .min(1, 'Give it a name')
    .max(NAME_MAX, `Keep it to ${NAME_MAX} characters`),
  body: bodySchema,
})

interface TemplateFormDialogProps {
  isOpen: boolean
  onOpenChange: (isOpen: boolean) => void
  /** The template to edit; `null` writes a new one. */
  template: Template | null
}

export function TemplateFormDialog({
  isOpen,
  onOpenChange,
  template,
}: TemplateFormDialogProps) {
  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      purpose="form"
      width={480}
    >
      {isOpen ? (
        <TemplateForm template={template} onClose={() => onOpenChange(false)} />
      ) : null}
    </Dialog>
  )
}

function TemplateForm({
  template,
  onClose,
}: {
  template: Template | null
  onClose: () => void
}) {
  const queryClient = useQueryClient()
  const [problem, setProblem] = useState<WriteProblem | null>(null)
  const failed = template
    ? "Couldn't save your changes."
    : "Couldn't save the template."

  const form = useForm({
    defaultValues: { name: template?.name ?? '', body: template?.body ?? '' },
    validators: { onSubmit: schema },
    canSubmitWhenInvalid: true,
    listeners: {
      onChange: () => setProblem(null),
    },
    onSubmit: async ({ value, formApi }) => {
      const body = { name: value.name.trim(), body: value.body.trim() }
      try {
        const result = template
          ? await api.PUT('/api/templates/{id}', {
              params: { path: { id: template.id } },
              body,
            })
          : await api.POST('/api/templates', { body })
        await queryClient.invalidateQueries({
          queryKey: templatesQuery.queryKey,
        })
        if (result.data) {
          onClose()
          return
        }
        const fields = toFieldErrors(result.error, ['name', 'body'] as const)
        if (fields) {
          formApi.setErrorMap({ onSubmit: { fields } })
        } else {
          setProblem(toWriteProblem(result, failed))
        }
      } catch {
        setProblem(connectionProblem(failed))
      }
    },
  })

  return (
    <DialogForm onSubmit={() => void form.handleSubmit()}>
      <Layout
        header={
          <DialogHeader
            title={template ? 'Edit template' : 'New template'}
            subtitle="Text you can drop into a message when you write one."
            onOpenChange={() => onClose()}
          />
        }
        content={
          <LayoutContent>
            <VStack gap={4}>
              {problem ? (
                <Banner
                  status="error"
                  title={problem.title}
                  description={problem.description}
                />
              ) : null}
              <FormLayout defaultOptionality="required">
                <form.Field name="name">
                  {(field) => (
                    <TextInput
                      label="Name"
                      size="lg"
                      placeholder="Friday recap"
                      htmlName={field.name}
                      value={field.state.value}
                      onChange={(value) => field.handleChange(value)}
                      status={toFieldStatus(field.state.meta)}
                    />
                  )}
                </form.Field>
                <form.Field name="body">
                  {(field) => (
                    <MessageBodyField
                      rows={6}
                      maxLength={BODY_MAX}
                      htmlName={field.name}
                      value={field.state.value}
                      onChange={(value) => field.handleChange(value)}
                      onBlur={field.handleBlur}
                      status={toFieldStatus(field.state.meta)}
                    />
                  )}
                </form.Field>
              </FormLayout>
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter>
            <HStack gap={2} justify="end">
              <Button
                label="Cancel"
                variant="secondary"
                clickAction={onClose}
              />
              <form.Subscribe selector={(state) => state.isSubmitting}>
                {(isSubmitting) => (
                  <Button
                    type="submit"
                    label={template ? 'Save changes' : 'Save template'}
                    variant="primary"
                    isLoading={isSubmitting}
                  />
                )}
              </form.Subscribe>
            </HStack>
          </LayoutFooter>
        }
      />
    </DialogForm>
  )
}
