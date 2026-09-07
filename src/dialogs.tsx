import { useEffect, useState } from 'react';

// WebView2 titles native alert()/confirm() dialogs with the page origin
// ("Tauri localhost"), which is meaningless to users. These in-app dialogs
// carry a proper product title instead.

const DEFAULT_TITLE = 'EasyTier Windows Client';

interface DialogRequest {
  title: string;
  message: string;
  confirm: boolean;
  resolve: (v: boolean) => void;
}

type PushFn = (r: { title: string; message: string; confirm: boolean }) => Promise<boolean>;
let push: PushFn | null = null;

/** In-app alert styled like the rest of the client. Resolves on dismiss. */
export const appAlert = (message: string, title = DEFAULT_TITLE): Promise<boolean> =>
  push ? push({ title, message, confirm: false }) : Promise.resolve(false);

/** In-app confirm; resolves true when the user accepts. */
export const appConfirm = (message: string, title = DEFAULT_TITLE): Promise<boolean> =>
  push ? push({ title, message, confirm: true }) : Promise.resolve(window.confirm(message));

export function DialogHost() {
  const [current, setCurrent] = useState<DialogRequest | null>(null);
  const [queue, setQueue] = useState<DialogRequest[]>([]);

  useEffect(() => {
    push = (r) => new Promise<boolean>(resolve => setQueue(q => [...q, { ...r, resolve }]));
    return () => { push = null; };
  }, []);

  // Show one dialog at a time; the rest wait in the queue.
  useEffect(() => {
    if (current || queue.length === 0) return;
    const [head, ...rest] = queue;
    setCurrent(head);
    setQueue(rest);
  }, [current, queue]);

  useEffect(() => {
    if (!current) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); current.resolve(false); setCurrent(null); }
      if (e.key === 'Enter') { e.preventDefault(); current.resolve(true); setCurrent(null); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [current]);

  if (!current) return null;
  const close = (v: boolean) => { current.resolve(v); setCurrent(null); };
  return (
    <div className="dialog-overlay" role="dialog" aria-modal="true" onMouseDown={e => { if (e.target === e.currentTarget && !current.confirm) close(false); }}>
      <div className="dialog-box">
        <h3>{current.title}</h3>
        <div className="dialog-message">
          {current.message.split('\n').map((line, i) => <p key={i}>{line || '\u00A0'}</p>)}
        </div>
        <div className="dialog-actions">
          {current.confirm && <button className="ghost" autoFocus={false} onClick={() => close(false)}>取消</button>}
          <button className="primary" onClick={() => close(true)}>{current.confirm ? '确定' : '好的'}</button>
        </div>
      </div>
    </div>
  );
}
