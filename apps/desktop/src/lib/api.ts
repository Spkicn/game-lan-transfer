// Tauri 命令与事件的类型化封装
//
// 前端不直接触碰文件系统与网络，一切都经这里转发到 Rust 侧
// VITE_MOCK=1 时改用假数据，便于在浏览器里单独检查界面

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import { mockEntries, mockGames, mockNetwork, mockPeers, mockProgressStep } from './mock';
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

/** 浏览器预览开关，生产构建里是常量 false */
const MOCK = import.meta.env.VITE_MOCK === '1';

/** 假数据的延迟返回 */
function mock<T>(value: T, ms = 160): Promise<T> {
  return new Promise((resolve) => {
    window.setTimeout(() => resolve(value), ms);
  });
}

/** 扫描本机游戏库 */
export function scanGames(): Promise<GameEntry[]> {
  if (MOCK) {
    return mock(mockGames);
  }
  return invoke<GameEntry[]>('scan_games');
}

/** 在直连网段里发现对端 */
export function discoverPeers(iface: string | null, seconds: number): Promise<PeerEntry[]> {
  if (MOCK) {
    return mock(mockPeers, 400);
  }
  return invoke<PeerEntry[]>('discover_peers', { iface, seconds });
}

/** 读取网卡与可用地址 */
export function networkStatus(): Promise<NetworkStatus> {
  if (MOCK) {
    return mock(mockNetwork);
  }
  return invoke<NetworkStatus>('network_status');
}

/** 配置直连地址 */
export function setupLink(host: number): Promise<string> {
  if (MOCK) {
    return mock(`192.168.88.${host}`);
  }
  return invoke<string>('setup_link', { host });
}

/** 还原直连配置 */
export function revertLink(): Promise<void> {
  if (MOCK) {
    return mock(undefined);
  }
  return invoke<void>('revert_link');
}

/** 列出本机目录内容；不传路径时给出盘符 */
export function listLocal(path: string | null): Promise<LocalEntry[]> {
  if (MOCK) {
    return mock(path === null ? mockEntries : mockEntries);
  }
  return invoke<LocalEntry[]>('list_local', { path });
}

/** 开始等待接收传输请求 */
export function startListen(iface: string | null, pairing: string | null): Promise<ListenInfo> {
  if (MOCK) {
    return mock({ addr: '192.168.88.1', port: 27102, code: pairing });
  }
  return invoke<ListenInfo>('start_listen', { iface, pairing });
}

/** 停止等待接收 */
export function stopListen(): Promise<void> {
  if (MOCK) {
    return mock(undefined);
  }
  return invoke<void>('stop_listen');
}

/** 回应一条传入请求 */
export function respondRequest(id: number, accepted: boolean, dest: string | null): Promise<void> {
  if (MOCK) {
    void id;
    void accepted;
    void dest;
    return mock(undefined);
  }
  return invoke<void>('respond_request', { id, accepted, dest });
}

/** 托管选中的内容并向对端发起请求 */
export function startSend(
  peer: string,
  items: string[],
  pairing: string | null,
  iface: string | null,
): Promise<SendInfo> {
  if (MOCK) {
    void peer;
    void items;
    void pairing;
    void iface;
    return mock({ port: 27101, approved: true, dest: 'D:\\Games', message: '' }, 500);
  }
  return invoke<SendInfo>('start_send', { peer, items, pairing, iface });
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
  if (MOCK) {
    return mock({
      root: dest,
      bytes_received: 64 * 1024 ** 3,
      bytes_total: 64 * 1024 ** 3,
      bytes_resumed: 60 * 1024 ** 3,
      claim_files: [],
    });
  }
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
  if (MOCK) {
    return mock(undefined);
  }
  return invoke<void>('cancel_recv');
}

/** 目标路径所在盘的可用空间 */
export function freeSpace(path: string): Promise<number> {
  if (MOCK) {
    return mock(240 * 1024 ** 3, 120);
  }
  return invoke<number>('free_space', { path });
}

/** 本机默认目标目录 */
export function defaultDest(): Promise<string | null> {
  if (MOCK) {
    return mock('D:\\SteamLibrary\\steamapps\\common');
  }
  return invoke<string | null>('default_dest');
}

/** 假请求，供预览时手动触发 */
export const mockRequest: IncomingEvent = {
  id: 1,
  from: '192.168.88.2:27101',
  sender_name: 'BAIUPC',
  total_bytes: 64 * 1024 ** 3,
  items: [
    { name: '示例游戏 A', is_dir: true, bytes: 64 * 1024 ** 3 },
    { name: '存档备份', is_dir: true, bytes: 812 * 1024 ** 2 },
  ],
  want: '示例游戏 A',
};

/** 订阅传输进度 */
export function onProgress(handler: (payload: Progress) => void): Promise<UnlistenFn> {
  if (MOCK) {
    let step = 0;
    const timer = window.setInterval(() => {
      step += 1;
      handler(mockProgressStep(step));
    }, 600);
    return Promise.resolve(() => window.clearInterval(timer));
  }
  return listen<Progress>('transfer://progress', (event) => {
    handler(event.payload);
  });
}

/** 订阅传入的传输请求 */
export function onRequest(handler: (payload: IncomingEvent) => void): Promise<UnlistenFn> {
  if (MOCK) {
    // 预览时用 window.__mockIncoming() 手动触发，5 秒后也会自动来一条
    (window as unknown as { __mockIncoming?: () => void }).__mockIncoming = () =>
      handler(mockRequest);
    window.setTimeout(() => handler(mockRequest), 5000);
    return Promise.resolve(() => undefined);
  }
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
