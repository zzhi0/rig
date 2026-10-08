// @vitest-environment jsdom
import '../test/setup';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import type { ApplicationInfo } from '../application/model';
import { ApplicationStore } from '../application/store';
import { App } from './App';

it('renders external store updates and delegates refresh to that store', async () => {
  let resolveFirst!: (info: ApplicationInfo) => void;
  let rejectSecond!: (error: Error) => void;
  const firstRequest = new Promise<ApplicationInfo>((resolve) => {
    resolveFirst = resolve;
  });
  const secondRequest = new Promise<ApplicationInfo>((_, reject) => {
    rejectSecond = reject;
  });
  const loadInfo = vi
    .fn<() => Promise<ApplicationInfo>>()
    .mockReturnValueOnce(firstRequest)
    .mockReturnValueOnce(secondRequest);
  const store = new ApplicationStore(loadInfo);
  const initialRefresh = store.refresh();
  render(<App store={store} />);

  expect(screen.getByRole('button', { name: 'Checking…' })).toBeDisabled();
  await act(async () => {
    resolveFirst({ name: 'Rig', version: '0.1.0' });
    await initialRefresh;
  });
  expect(screen.getByText('0.1.0')).toBeInTheDocument();

  fireEvent.click(screen.getByRole('button', { name: 'Refresh connection' }));
  expect(loadInfo).toHaveBeenCalledTimes(2);
  expect(screen.getByRole('button', { name: 'Checking…' })).toBeDisabled();
  expect(screen.queryByText('0.1.0')).not.toBeInTheDocument();

  await act(async () => {
    rejectSecond(new Error('The local service returned HTTP 503.'));
    await secondRequest.catch(() => {});
  });
  expect(
    screen.getByText('The local service returned HTTP 503.'),
  ).toBeInTheDocument();
  expect(
    screen.getByRole('button', { name: 'Refresh connection' }),
  ).toBeEnabled();
});
