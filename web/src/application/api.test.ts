import { afterEach, describe, expect, it, vi } from 'vitest';
import { fetchApplicationInfo } from './api';

afterEach(() => vi.unstubAllGlobals());

describe('fetchApplicationInfo', () => {
  it('reads application information using a same-origin request', async () => {
    const info = { name: 'Rig', version: '0.1.0' };
    const fetchMock = vi.fn().mockResolvedValue(Response.json(info));
    vi.stubGlobal('fetch', fetchMock);

    await expect(fetchApplicationInfo()).resolves.toEqual(info);
    expect(fetchMock).toHaveBeenCalledWith('/api/info');
  });

  it('reports HTTP errors rather than treating their bodies as application data', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response('', { status: 503 })),
    );
    await expect(fetchApplicationInfo()).rejects.toThrow(
      'The local service returned HTTP 503.',
    );
  });

  it('propagates network failures to the state owner', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockRejectedValue(new TypeError('Failed to fetch')),
    );
    await expect(fetchApplicationInfo()).rejects.toThrow('Failed to fetch');
  });
});
