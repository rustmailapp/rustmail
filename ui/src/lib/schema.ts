import * as z from "zod/mini";

/**
 * The response shapes `docs/api.yaml` promises, as runtime checks.
 *
 * These are what `types.ts` infers the domain types from, so a schema and the
 * type the UI compiles against cannot drift apart. `zod/mini` rather than the
 * full build: this bundle is embedded in the binary, and mini costs a fifth of
 * what the standard build does for the same schemas.
 */

export const messageSummary = z.object({
  id: z.string(),
  sender: z.string(),
  recipients: z.array(z.string()),
  subject: z.nullable(z.string()),
  size: z.number(),
  has_attachments: z.boolean(),
  is_read: z.boolean(),
  is_starred: z.boolean(),
  tags: z.array(z.string()),
  created_at: z.string(),
});

export const message = z.extend(messageSummary, {
  text_body: z.nullable(z.string()),
  html_body: z.nullable(z.string()),
});

export const listResponse = z.object({
  messages: z.array(messageSummary),
  total: z.number(),
});

export const attachment = z.object({
  id: z.string(),
  message_id: z.string(),
  filename: z.nullable(z.string()),
  content_type: z.nullable(z.string()),
  content_id: z.nullable(z.string()),
  size: z.nullable(z.number()),
});

export const attachmentList = z.array(attachment);

export const messageHeader = z.object({
  name: z.string(),
  value: z.string(),
});

export const headerList = z.array(messageHeader);

export const authCheck = z.object({
  status: z.string(),
  details: z.string(),
});

export const authResults = z.object({
  dkim: z.array(authCheck),
  spf: z.array(authCheck),
  dmarc: z.array(authCheck),
  arc: z.array(authCheck),
});

const identified = z.object({ id: z.string() });

/** Every event the server broadcasts, discriminated by `type`. */
export const wsEvent = z.discriminatedUnion("type", [
  z.object({ type: z.literal("message:new"), data: messageSummary }),
  z.object({ type: z.literal("message:delete"), data: identified }),
  z.object({
    type: z.literal("message:read"),
    data: z.extend(identified, { is_read: z.boolean() }),
  }),
  z.object({
    type: z.literal("message:starred"),
    data: z.extend(identified, { is_starred: z.boolean() }),
  }),
  z.object({
    type: z.literal("message:tags"),
    data: z.extend(identified, { tags: z.array(z.string()) }),
  }),
  z.object({ type: z.literal("messages:clear") }),
]);
