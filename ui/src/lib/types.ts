import type * as z from "zod/mini";
import type * as schema from "./schema";

/**
 * The domain types, inferred from the schemas that check them at the boundary.
 *
 * Declaring these by hand alongside the schemas would let the two drift, and
 * the drift would only show as a runtime rejection of a response the server
 * was right to send. `FilterState` is written out because it is inbox state
 * that never crosses a boundary, so nothing validates it.
 */

export type MessageSummary = z.infer<typeof schema.messageSummary>;
export type Message = z.infer<typeof schema.message>;
export type Attachment = z.infer<typeof schema.attachment>;
export type ListResponse = z.infer<typeof schema.listResponse>;
export type MessageHeader = z.infer<typeof schema.messageHeader>;
export type AuthCheck = z.infer<typeof schema.authCheck>;
export type AuthResults = z.infer<typeof schema.authResults>;
export type WsEvent = z.infer<typeof schema.wsEvent>;

export interface FilterState {
  starred: boolean;
  unread: boolean;
  attachments: boolean;
  tags: string[];
}
