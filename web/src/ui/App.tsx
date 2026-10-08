import { useSyncExternalStore } from 'react';
import type { ApplicationStore } from '../application/store';

export function App({ store }: { store: ApplicationStore }) {
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const loading = state.phase === 'idle' || state.phase === 'loading';
  const connected = state.phase === 'ready';

  return (
    <div className="shell">
      <header className="masthead">
        <a className="wordmark" href="/" aria-label="Rig home">
          Rig
          <span className="wordmark-dot" aria-hidden="true" />
        </a>
        <span className="masthead-description">Personal agent harness</span>
      </header>

      <main>
        <div className="page-heading">
          <h1>Your local workspace</h1>
          <p>Rig runs on your machine. Check your service connection here.</p>
        </div>

        <section className="connection" aria-labelledby="connection-title">
          <div className="connection-heading">
            <h2 id="connection-title">Service connection</h2>
            <button onClick={() => void store.refresh()} disabled={loading}>
              {loading ? 'Checking…' : 'Refresh connection'}
            </button>
          </div>

          <div className="connection-status" role="status" aria-live="polite">
            <span
              className={`status-dot ${connected ? 'connected' : loading ? 'checking' : 'unavailable'}`}
              aria-hidden="true"
            />
            <div>
              <h3>
                {connected
                  ? 'Connected to Rig'
                  : loading
                    ? 'Connecting to your service'
                    : 'Service unavailable'}
              </h3>
              <p>
                {connected
                  ? 'Your local service is responding.'
                  : loading
                    ? 'Reading application information.'
                    : 'Check that Rig is running, then refresh the connection.'}
              </p>
            </div>
          </div>

          {state.phase === 'ready' && (
            <dl className="application-details">
              <div>
                <dt>Application</dt>
                <dd>{state.info.name}</dd>
              </div>
              <div>
                <dt>Version</dt>
                <dd>{state.info.version}</dd>
              </div>
            </dl>
          )}
          {state.phase === 'error' && (
            <p className="error-detail">{state.message}</p>
          )}
        </section>
      </main>

      <footer>Local first. Yours to build on.</footer>
    </div>
  );
}
