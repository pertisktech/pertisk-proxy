import { useEffect, useRef } from 'react';
import {
  setLiveLogsFilter,
  subscribeLive,
  type LiveChannel,
  type LogsFilter,
} from '@/api/live';

type Options<T> = {
  enabled?: boolean;
  onData: (data: T) => void;
  logsFilter?: LogsFilter;
};

/**
 * Subscribe to a live WebSocket channel while `enabled` is true.
 * Keeps the latest `onData` via ref so the socket subscription is stable.
 */
export function useLiveChannel<T = unknown>(
  channel: LiveChannel,
  { enabled = true, onData, logsFilter }: Options<T>,
) {
  const onDataRef = useRef(onData);
  onDataRef.current = onData;

  useEffect(() => {
    if (!enabled) return;
    return subscribeLive(channel, (data) => {
      onDataRef.current(data as T);
    });
  }, [channel, enabled]);

  useEffect(() => {
    if (!enabled || channel !== 'logs' || !logsFilter) return;
    setLiveLogsFilter(logsFilter);
  }, [channel, enabled, logsFilter?.type, logsFilter?.host]);
}
