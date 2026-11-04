import type {
  CommentTag,
  CreatedSession,
  Draft,
  ReviewPayload,
  SessionSummary,
  Side,
  Suggestion,
} from './types.ts';

const token = new URLSearchParams(window.location.search).get('token') ?? '';

export function eventsUrl(session: string | null): string {
  const params = new URLSearchParams({ token });
  if (session) params.set('session', session);
  return `/api/events?${params.toString()}`;
}

export class GoneError extends Error {}

let current: string | null = null;

/** Which session the session-scoped calls below talk to. */
export function setApiSession(id: string | null): void {
  current = id;
}

function scoped(path: string): string {
  if (!current) throw new Error('No session is selected.');
  return `/api/sessions/${encodeURIComponent(current)}${path}`;
}

async function send<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    headers: {
      'x-review-token': token,
      ...(body === undefined ? {} : { 'content-type': 'application/json' }),
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });

  if (!res.ok) {
    if (res.status === 403) {
      throw new Error('This tab is missing its session token. Reopen the URL claude-review printed.');
    }

    const detail = await res
      .json()
      .then((d: { error?: string }) => d.error)
      .catch(() => '');

    if (res.status === 410) throw new GoneError(detail || 'This session’s worktree no longer exists.');
    throw new Error(detail || `Request to ${path} failed (${res.status}).`);
  }

  return (await res.json()) as T;
}

export function fetchReview(): Promise<ReviewPayload> {
  return send<ReviewPayload>(scoped('/review'));
}

export interface NewCommentInput {
  path: string;
  side: Side | null;
  startLine: number | null;
  endLine: number | null;
  tag: CommentTag;
  body: string;
}

export function createComment(input: NewCommentInput): Promise<Draft> {
  return send<Draft>(scoped('/comments'), 'POST', input);
}

export function updateComment(id: string, patch: { tag?: CommentTag; body?: string }): Promise<Draft> {
  return send<Draft>(scoped(`/comments/${encodeURIComponent(id)}`), 'PATCH', patch);
}

export function deleteComment(id: string): Promise<Draft> {
  return send<Draft>(scoped(`/comments/${encodeURIComponent(id)}`), 'DELETE');
}

export function putOverall(overall: string): Promise<Draft> {
  return send<Draft>(scoped('/overall'), 'PUT', { overall });
}

export interface FileContent {
  path: string;
  side: Side;
  content: string | null;
}

export function fetchFile(path: string, side: Side): Promise<FileContent> {
  return send<FileContent>(scoped(`/file?path=${encodeURIComponent(path)}&side=${side}`));
}

export interface SubmitResult {
  markdownPath: string;
  jsonPath: string;
  latestMarkdownPath: string;
  commentCount: number;
  markdown: string;
}

export function fetchPreview(): Promise<{ markdown: string }> {
  return send<{ markdown: string }>(scoped('/preview'));
}

export function submitReview(): Promise<SubmitResult> {
  return send<SubmitResult>(scoped('/submit'), 'POST', {});
}

export function putViewed(path: string, digest: string | null): Promise<Draft> {
  return send<Draft>(scoped('/viewed'), 'PUT', { path, digest });
}

export function listSessions(): Promise<SessionSummary[]> {
  return send<SessionSummary[]>('/api/sessions');
}

export interface NewSessionInput {
  path: string;
  name?: string;
  base?: string;
}

export function createSession(input: NewSessionInput): Promise<CreatedSession> {
  return send<CreatedSession>('/api/sessions', 'POST', input);
}

export function updateSession(
  id: string,
  patch: { name?: string; base?: string; status?: 'open' | 'closed' },
): Promise<SessionSummary> {
  return send<SessionSummary>(`/api/sessions/${encodeURIComponent(id)}`, 'PATCH', patch);
}

export function closeSession(id: string): Promise<SessionSummary> {
  return send<SessionSummary>(`/api/sessions/${encodeURIComponent(id)}`, 'DELETE');
}

export function fetchSuggestions(): Promise<Suggestion[]> {
  return send<Suggestion[]>('/api/suggestions');
}
