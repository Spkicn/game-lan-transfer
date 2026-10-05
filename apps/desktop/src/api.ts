// Tauri 命令与事件的类型化封装
//
// 前端不直接触碰文件系统与网络，一切都经这里转发到 Rust 侧

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import type {
  GameEntry,
  HostInfo,
  NetworkStatus,
  PeerEntry,
  Preview,
  Progress,
  RecvSummary,
} from './state';

/** 扫描本机游戏库 */
export function scanGames(): Promise<GameEntry[]> {
  return invoke<GameEntry[]>('scan_games');
}

/** 在直连网段里发现对端 */
export function discoverPeers(iface: string | null, seconds: number): Promise<PeerEntry[]> {
  return invoke<PeerEntry[]>('discover_peers', { iface, seconds });
}

/** 读取网卡与可用地址 */
export function networkStatus(): Promise<NetworkStatus> {
  return invoke<NetworkStatus>('network_status');
}

/** 给物理以太网口配置直连地址 */
export function setupLink(host: number): Promise<string> {
  return invoke<string>('setup_link', { host });
}

/** 还原直连配置 */
export function revertLink(): Promise<void> {
  return invoke<void>('revert_link');
}

/** 传输前预检 */
export function previewTarget(
  peer: string,
  want: string,
  dest: string,
  pairing: string | null,
): Promise<Preview> {
  return invoke<Preview>('preview_target', { peer, want, dest, pairing });
}

/** 启动源端服务 */
export function startHost(
  installDir: string,
  title: string,
  platform: string,
  code: string | null,
  iface: string | null,
): Promise<HostInfo> {
  return invoke<HostInfo>('start_host', { installDir, title, platform, code, iface });
}

/** 停止源端服务 */
export function stopHost(): Promise<void> {
  return invoke<void>('stop_host');
}

/** 接收内容 */
export function startRecv(
  peer: string,
  want: string,
  dest: string,
  platform: string | null,
  pairing: string | null,
  force: boolean,
): Promise<RecvSummary> {
  return invoke<RecvSummary>('start_recv', { peer, want, dest, platform, pairing, force });
}

/** 取消接收 */
export function cancelRecv(): Promise<void> {
  return invoke<void>('cancel_recv');
}

/** 本机默认目标目录 */
export function defaultDest(): Promise<string | null> {
  return invoke<string | null>('default_dest');
}

/** 订阅传输进度事件 */
export function onProgress(handler: (payload: Progress) => void): Promise<UnlistenFn> {
  return listen<Progress>('transfer://progress', (event) => {
    handler(event.payload);
  });
}

/** 把命令抛出的错误转成可展示的文本 */
export function describeError(error: unknown): string {
  if (typeof error === 'string') {
    return error;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}
