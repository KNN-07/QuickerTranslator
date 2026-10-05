import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invokeCommand, nativeAvailable, toAppError } from '../../lib/ipc';
import type { AppError } from '../../lib/types';
import type { ShortcutRecord } from '../../lib/dictionaryTypes';

export function useShortcuts(onError: (error: AppError) => void) {
  const [records, setRecords] = useState<ShortcutRecord[]>([]);
  useEffect(() => {
    if (!nativeAvailable()) return;
    let disposed = false;
    let generation = 0;
    const reload = async () => {
      const ticket = ++generation;
      try { const result = await invokeCommand<ShortcutRecord[]>('list_shortcuts'); if (!disposed && ticket === generation) setRecords(result); }
      catch (error: unknown) { if (!disposed && ticket === generation) onError(toAppError(error)); }
    };
    void reload();
    const subscription = listen('dictionaries-changed', () => { void reload(); });
    void subscription.catch((error: unknown) => { if (!disposed) onError(toAppError(error)); });
    return () => { disposed = true; void subscription.then((unlisten) => unlisten()).catch(() => {}); };
  }, [onError]);
  return records;
}
