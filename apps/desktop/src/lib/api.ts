// Tauri 命令与事件的类型化封装
//
// 前端不直接触碰文件系统与网络，一切都经这里转发到 Rust 侧

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import type {
  GameEntry,
  IncomingEvent,
  ListenInfo,
  LocalEntry,
  NetworkStatus,
  PeerEntry,
  Progress,
  RecvSummary,
  SendInfo,
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

/** 配置直连地址 */
export function setupLink(host: number): Promise<string> {
  return invoke<string>('setup_link', { host });
}

/** 还原直连配置 */
export function revertLink(): Promise<void> {
  return invoke<void>('revert_link');
}

/** 列出本机目录内容；不传路径时给出盘符 */
export function listLocal(path: string | null): Promise<LocalEntry[]> {
  return invoke<LocalEntry[]>('list_local', { path });
}

/** 开始等待接收传输请求 */
export function startListen(iface: string | null, pairing: string | null): Promise<ListenInfo> {
  return invoke<ListenInfo>('start_listen', { iface, pairing });
}

/** 停止等待接收 */
export function stopListen(): Promise<void> {
  return invoke<void>('stop_listen');
}

/** 回应一条传入请求 */
export function respondRequest(id: number, accepted: boolean, dest: string | null): Promise<void> {
  return invoke<void>('respond_request', { id, accepted, dest });
}

/** 托管选中的内容并向对端发起请求 */
export function startSend(
  peer: string,
  items: string[],
  pairing: string | null,
  iface: string | null,
): Promise<SendInfo> {
  return invoke<SendInfo>('start_send', { peer, items, pairing, iface });
}

/** 源端已下发字节数 */
export function sendProgress(): Promise<number> {
  return invoke<number>('send_progress');
}

/** 接收内容 */
export function startRecv(
  peer: string,
  want: string,
  dest: string,
  platform: string | null,
  pairing: string | null,
  force: boolean,
  intoDestination: boolean,
): Promise<RecvSummary> {
  return invoke<RecvSummary>('start_recv', {
    peer,
    want,
    dest,
    platform,
    pairing,
    force,
    intoDestination,
  });
}

/** 取消接收 */
export function cancelRecv(): Promise<void> {
  return invoke<void>('cancel_recv');
}

/** 本机默认目标目录 */
export function defaultDest(): Promise<string | null> {
  return invoke<string | null>('default_dest');
}

/** 订阅传输进度 */
export function onProgress(handler: (payload: Progress) => void): Promise<UnlistenFn> {
  return listen<Progress>('transfer://progress', (event) => {
    handler(event.payload);
  });
}

/** 订阅传入的传输请求 */
export function onRequest(handler: (payload: IncomingEvent) => void): Promise<UnlistenFn> {
  return listen<IncomingEvent>('transfer://request', (event) => {
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
