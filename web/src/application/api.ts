import type { ApplicationInfo } from './model';

export async function fetchApplicationInfo(): Promise<ApplicationInfo> {
  const response = await fetch('/api/info');
  if (!response.ok) {
    throw new Error(`The local service returned HTTP ${response.status}.`);
  }
  return response.json() as Promise<ApplicationInfo>;
}
