import React, { useState, useEffect } from 'react';
import { ConfigEditor } from '../../ConfigEditor';
import { decodeTOML, encodeTOML } from '../../toml-codec';
import { listenersForInstance, NetworkConfig, validateConfig } from '../../network-config';
import { Instance } from '../../types';
import { IconClipboard, IconCopy, IconDownload, IconTrash, IconUpload } from '../../icons';
import { appAlert, appConfirm } from '../../dialogs';

interface ConfigTabProps {
  current: Instance;
  instances: Instance[];
  configSaved: boolean;
  renameInstance: (name: string) => void;
  removeInstance: (id: string) => void;
  patchConfig: (patch: Partial<NetworkConfig>) => void;
  showAdvanced: boolean;
  setShowAdvanced: React.Dispatch<React.SetStateAction<boolean>>;
  secretVisible: boolean;
  addLog: (msg: string) => void;
  showToast: (msg: string) => void;
  copyText: (text: string, label?: string) => Promise<void>;
  setInstances: React.Dispatch<React.SetStateAction<Instance[]>>;
}

export const ConfigTab: React.FC<ConfigTabProps> = ({
  current,
  instances,
  configSaved,
  renameInstance,
  removeInstance,
  patchConfig,
  showAdvanced,
  setShowAdvanced,
  secretVisible,
  addLog,
  showToast,
  copyText,
  setInstances,
}) => {
  const [tomlDraft, setTomlDraft] = useState<string | null>(null);
  const [tomlDirty, setTomlDirty] = useState(false);
  const [tomlError, setTomlError] = useState<string | null>(null);

  // 切换不同实例时，强制重置 TOML 编辑草稿与未保存状态，杜绝跨实例配置串改
  useEffect(() => {
    setTomlDraft(null);
    setTomlDirty(false);
    setTomlError(null);
  }, [current?.id]);

  useEffect(() => {
    if (tomlDraft !== null && !tomlDirty) {
      setTomlDraft(encodeTOML(current.config));
    }
  }, [current.config, tomlDraft, tomlDirty]);

  const importToml = async (text: string) => {
    try {
      const config = decodeTOML(text);
      // 保证导入的配置严格采用当前实例自己的独立 ID，防止复制配置导致 EasyTier 节点 ID 冲突
      config.instance_id = current.id;
      const sameListeners =
        config.listener_urls.length === current.config.listener_urls.length &&
        config.listener_urls.every((u, i) => u === current.config.listener_urls[i]);
      if (sameListeners) {
        const instIndex = Math.max(0, instances.findIndex(i => i.id === current.id));
        config.listener_urls = listenersForInstance(instIndex);
      }
      patchConfig(config);
      addLog(`TOML 配置导入成功（监听器 ${config.listener_urls.join(', ')}）`);
      await appAlert(
        `配置导入成功\n\n监听器：${config.listener_urls.join('\n')}\n\n若与其它实例端口冲突，请在监听器列表中修改端口后重新启动网络。`
      );
    } catch (e) {
      addLog(`TOML 导入失败：${String(e)}`);
      await appAlert(`导入失败：${String(e)}`);
    }
  };

  const exportToml = async (copy = false) => {
    const hasSecret = !!current.config.network_secret;
    if (copy && hasSecret && !secretVisible) {
      if (!(await appConfirm('配置中包含网络密钥，确定复制到剪贴板吗？'))) return;
    }
    // 导出用于跨节点/跨实例分享的 TOML 时，跳过 instance_id，避免导入时产生节点 ID 冲突
    const text = encodeTOML(current.config, true, true);
    try {
      if (copy) {
        await copyText(text, 'TOML 已复制到剪贴板');
      } else {
        const a = document.createElement('a');
        a.href = URL.createObjectURL(new Blob([text], { type: 'application/toml' }));
        a.download = `${current.name || 'easytier'}.toml`;
        a.click();
        URL.revokeObjectURL(a.href);
        showToast('✓ TOML 已导出为文件');
      }
      addLog(copy ? 'TOML 已复制到剪贴板' : 'TOML 已导出为文件');
    } catch (e) {
      await appAlert(`导出失败：${String(e)}`);
    }
  };

  const openTomlFile = async () => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.toml,text/plain';
    input.onchange = async () => {
      const f = input.files?.[0];
      if (f) await importToml(await f.text());
    };
    input.click();
  };

  const startTomlEdit = () => {
    if (tomlDraft === null || !tomlDirty) setTomlDraft(encodeTOML(current.config));
    setTomlDirty(false);
    setTomlError(null);
  };

  const applyTomlDraft = () => {
    if (tomlDraft === null) return;
    try {
      const config = decodeTOML(tomlDraft);
      const errors = validateConfig(config);
      if (errors.length) throw new Error(errors[0].message);
      // 若草稿中的 instance_id 与其它实例发生冲突或为空，自动回退到当前实例自己的唯一 ID
      const isConflicting = instances.some(
        other => other.id !== current.id && (other.id === config.instance_id || other.config.instance_id === config.instance_id)
      );
      if (!config.instance_id || isConflicting) {
        config.instance_id = current.id;
      }
      setInstances(xs => xs.map(i => (i.id === current.id ? { ...i, config } : i)));
      setTomlDraft(null);
      setTomlError(null);
      addLog(`[${current.name}] TOML 已应用到表单`);
    } catch (e) {
      setTomlError(String(e instanceof Error ? e.message : e));
    }
  };

  const revertTomlDraft = () => {
    setTomlDraft(encodeTOML(current.config));
    setTomlError(null);
  };

  return (
    <div className="card config-card-wide">
      <div className="card-title-row">
        <input className="title-input" value={current.name} onChange={e => renameInstance(e.target.value)} />
        {configSaved && (
          <span className="save-status" role="status">
            ✓ 已自动保存
          </span>
        )}
        <div className="title-actions">
          <button className="ghost" onClick={() => void openTomlFile()}>
            <IconUpload size={13} /> 导入 TOML
          </button>
          <button
            className="ghost"
            onClick={async () => {
              try {
                await importToml(await navigator.clipboard.readText());
              } catch (e) {
                await appAlert(`读取剪贴板失败：${String(e)}`);
              }
            }}
          >
            <IconClipboard size={13} /> 剪贴板导入
          </button>
          <button className="ghost" onClick={() => void exportToml()}>
            <IconDownload size={13} /> 导出 TOML
          </button>
          <button className="ghost" onClick={() => void exportToml(true)}>
            <IconCopy size={13} /> 复制
          </button>
          <button className="mini-button danger" onClick={() => removeInstance(current.id)}>
            <IconTrash size={12} /> 删除实例
          </button>
        </div>
      </div>
      <ConfigEditor
        config={current.config}
        onChange={patchConfig}
        showAdvanced={showAdvanced}
        onToggleAdvanced={() => setShowAdvanced(x => !x)}
      />
      <details
        className="toml-preview"
        onToggle={e => {
          if ((e.target as HTMLDetailsElement).open) startTomlEdit();
        }}
      >
        <summary>查看当前 TOML（可编辑）</summary>
        <textarea
          className="toml-editor"
          value={tomlDraft ?? encodeTOML(current.config)}
          onChange={e => {
            setTomlDraft(e.target.value);
            setTomlDirty(true);
          }}
          spellCheck={false}
          rows={18}
        />
        {tomlError && <p className="toml-error">✗ {tomlError}</p>}
        <div className="toml-actions">
          <button className="primary" onClick={applyTomlDraft}>
            应用更改
          </button>
          <button className="ghost" onClick={revertTomlDraft}>
            还原
          </button>
        </div>
      </details>
    </div>
  );
};
