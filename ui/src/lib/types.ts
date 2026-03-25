export interface MessageSummary {
  id: string;
  sender: string;
  recipients: string[];
  subject: string | null;
  size: number;
  has_attachments: boolean;
  is_read: boolean;
  is_starred: boolean;
  tags: string[];
  created_at: string;
}

export interface Message extends MessageSummary {
  text_body: string | null;
  html_body: string | null;
}

export interface Attachment {
  id: string;
  message_id: string;
  filename: string | null;
  content_type: string | null;
  content_id: string | null;
  size: number | null;
}

export interface ListResponse {
  messages: MessageSummary[];
  total: number;
}

export interface AuthCheck {
  status: string;
  details: string;
}

export interface AuthResults {
  dkim: AuthCheck[];
  spf: AuthCheck[];
  dmarc: AuthCheck[];
  arc: AuthCheck[];
}

export interface FilterState {
  starred: boolean;
  unread: boolean;
  attachments: boolean;
  tags: string[];
}

export type WsEvent =
  | { type: "message:new"; data: MessageSummary }
  | { type: "message:delete"; data: { id: string } }
  | { type: "message:read"; data: { id: string; is_read: boolean } }
  | { type: "message:starred"; data: { id: string; is_starred: boolean } }
  | { type: "message:tags"; data: { id: string; tags: string[] } }
  | { type: "messages:clear" };
