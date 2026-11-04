import { useEffect } from 'react';
import { eventsUrl } from './api.ts';
import { useReview } from './store.ts';

/**
 * Follows the server's events: `sessions` when any session changes, including
 * ones Claude registers from the CLI, and `changed` when the open session's
 * working tree does.
 */
export function useLiveReload(): void {
  const sessionId = useReview((s) => s.sessionId);

  useEffect(() => {
    const source = new EventSource(eventsUrl(sessionId));

    source.addEventListener('sessions', () => {
      void useReview.getState().loadSessions();
    });

    source.addEventListener('changed', () => {
      const store = useReview.getState();
      const busy =
        store.selection !== null ||
        store.preview !== null ||
        store.composingPath !== null ||
        store.submitting;

      if (busy) store.setStale(true);
      else void store.load();
    });

    source.onerror = () => {};

    return () => source.close();
  }, [sessionId]);
}
