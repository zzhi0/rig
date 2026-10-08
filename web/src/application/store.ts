import type { ApplicationInfo, ApplicationState } from './model';

type Listener = () => void;

export class ApplicationStore {
  private state: ApplicationState = { phase: 'idle' };
  private readonly listeners = new Set<Listener>();

  constructor(private readonly loadInfo: () => Promise<ApplicationInfo>) {}

  getSnapshot = (): ApplicationState => this.state;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  refresh = async (): Promise<void> => {
    this.publish({ phase: 'loading' });
    try {
      const info = await this.loadInfo();
      this.publish({ phase: 'ready', info });
    } catch (error) {
      this.publish({ phase: 'error', message: (error as Error).message });
    }
  };

  private publish(state: ApplicationState): void {
    this.state = state;
    this.listeners.forEach((listener) => listener());
  }
}
