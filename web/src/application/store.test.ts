import { describe, expect, it, vi } from 'vitest';
import type { ApplicationInfo, ApplicationState } from './model';
import { ApplicationStore } from './store';

const info: ApplicationInfo = { name: 'Rig', version: '0.1.0' };

describe('ApplicationStore', () => {
  it('publishes loading and server data without a UI dependency', async () => {
    let resolve!: (info: ApplicationInfo) => void;
    const request = new Promise<ApplicationInfo>((resolveRequest) => {
      resolve = resolveRequest;
    });
    const store = new ApplicationStore(() => request);
    const states: ApplicationState[] = [];
    const unsubscribe = store.subscribe(() => states.push(store.getSnapshot()));

    expect(store.getSnapshot()).toEqual({ phase: 'idle' });
    const refresh = store.refresh();
    expect(states).toEqual([{ phase: 'loading' }]);

    resolve(info);
    await refresh;
    expect(states).toEqual([{ phase: 'loading' }, { phase: 'ready', info }]);

    unsubscribe();
    await store.refresh();
    expect(states).toHaveLength(2);
  });

  it('publishes a request failure and allows a deliberate refresh', async () => {
    const loadInfo = vi
      .fn<() => Promise<ApplicationInfo>>()
      .mockRejectedValueOnce(new Error('The local service returned HTTP 503.'))
      .mockResolvedValueOnce(info);
    const store = new ApplicationStore(loadInfo);

    await store.refresh();
    expect(store.getSnapshot()).toEqual({
      phase: 'error',
      message: 'The local service returned HTTP 503.',
    });
    expect(loadInfo).toHaveBeenCalledTimes(1);

    await store.refresh();
    expect(store.getSnapshot()).toEqual({ phase: 'ready', info });
    expect(loadInfo).toHaveBeenCalledTimes(2);
  });
});
