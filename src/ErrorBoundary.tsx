import { Component, ReactNode } from 'react';

interface State { error: Error | null }

// Last-resort guard: without it an uncaught render error unmounts the whole
// tree and the window goes blank (React 18 production behavior).
export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div style={{ margin: '3rem auto', maxWidth: 640, fontFamily: 'sans-serif' }}>
        <h2>界面渲染出错</h2>
        <pre style={{ whiteSpace: 'pre-wrap', background: '#f6f6f6', padding: '1rem', borderRadius: 8 }}>{String(error.stack || error)}</pre>
        <button type="button" onClick={() => this.setState({ error: null })}>重试</button>
        <button
          type="button"
          style={{ marginLeft: 12 }}
          onClick={() => {
            for (const k of Object.keys(localStorage)) if (k.startsWith('easytier.')) localStorage.removeItem(k);
            location.reload();
          }}
        >
          重置所有本地数据并重载
        </button>
      </div>
    );
  }
}
