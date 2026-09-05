import { invoke } from '@tauri-apps/api/core';

export type ServiceCommand =
  | 'service_status'
  | 'list_instances'
  | 'sync_instance'
  | 'remove_instance'
  | 'start_instance'
  | 'stop_instance'
  | 'set_auto_start'
  | 'update_kernel'
  | 'run_cli'
  | 'get_task_status'
  | 'install_service'
  | 'start_service'
  | 'repair_service'
  | 'get_network_logs';

export interface ServiceRequest<T = Record<string, unknown>> {
  protocol_version: 1;
  request_id: string;
  command: ServiceCommand;
  payload: T;
}

export interface ServiceResponse<T = unknown> {
  protocol_version: number;
  request_id: string;
  ok: boolean;
  data?: T;
  error?: { code?: string; message?: string } | string | null;
}

export interface ServiceStatus {
  installed: boolean;
  running: boolean;
  healthy?: boolean;
  version?: string | null;
  message?: string | null;
}

export interface ServiceInstanceState {
  id: string;
  name?: string;
  config_toml?: string;
  rpc_port?: number;
  auto_start: boolean;
  desired_state: 'stopped' | 'running';
  observed_state: 'stopped' | 'starting' | 'running' | 'stopping' | 'failed';
  last_error?: string | null;
  remote_manage_enabled?: boolean;
  rpc_whitelist_cidrs?: string[];
}

export async function serviceRequest<TData = unknown, TPayload = Record<string, unknown>>(
  command: ServiceCommand,
  payload = {} as TPayload,
): Promise<TData> {
  const request: ServiceRequest<TPayload> = {
    protocol_version: 1,
    request_id: crypto.randomUUID(),
    command,
    payload,
  };
  const response = await invoke<ServiceResponse<TData>>('service_request', { request });
  if (!response?.ok) {
    const error = response?.error;
    const message = typeof error === 'string' ? error : error?.message || error?.code || '服务请求失败';
    throw new Error(message);
  }
  return response.data as TData;
}

export async function getServiceStatus(): Promise<ServiceStatus> {
  return serviceRequest<ServiceStatus>('service_status');
}
