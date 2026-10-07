import { useCallback, useEffect, useState } from 'react';

export interface AsyncState<T> {
  data: T | null;
  loading: boolean;
  error: unknown;
  reload: () => void;
}

/**
 * Runs `load` on mount and whenever its identity changes (build it with
 * `useCallback`). A stale response from a superseded call is dropped.
 */
export function useAsyncData<T>(load: () => Promise<T>): AsyncState<T> {
  const [state, setState] = useState<{ data: T | null; loading: boolean; error: unknown }>({
    data: null,
    loading: true,
    error: null,
  });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setState(prev => ({ ...prev, loading: true, error: null }));
    load().then(
      data => {
        if (!cancelled) setState({ data, loading: false, error: null });
      },
      error => {
        if (!cancelled) setState({ data: null, loading: false, error: error ?? new Error('') });
      }
    );
    return () => {
      cancelled = true;
    };
  }, [load, attempt]);

  const reload = useCallback(() => setAttempt(n => n + 1), []);
  return { ...state, reload };
}
