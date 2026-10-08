export interface ApplicationInfo {
  name: string;
  version: string;
}

export type ApplicationState =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'ready'; info: ApplicationInfo }
  | { phase: 'error'; message: string };
