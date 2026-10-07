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
  LinkRole,
  ListenInfo,
  LocalEntry,
  NetworkStatus,
  PeerEntry,
  Progress,
  RecvSummary,
  RoleSetup,
  SteamLibrary,
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

/** 按角色配置直连地址，发送端用 .1，接收端用 .2 */
export function configureRole(role: LinkRole): Promise<RoleSetup> {
  if (MOCK) {
    return mock(
      {
        nic_name: '以太网',
        ip: role === 'sender' ? '192.168.88.1' : '192.168.88.2',
        already: false,
      },
      300,
    );
  }
  return invoke<RoleSetup>('configure_role', { role });
}

/** 还原直连配置，两种角色的地址都清理 */
export function resetLink(): Promise<void> {
  if (MOCK) {
    return mock(undefined);
  }
  return invoke<void>('reset_link');
}

/** 当前程序版本，诊断信息里带上 */
export function appVersion(): Promise<string> {
  if (MOCK) {
    return mock('0.2.3');
  }
  return invoke<string>('app_version');
}

/** 当前是否以管理员身份运行，探测不出来时为 null */
export function elevationStatus(): Promise<boolean | null> {
  if (MOCK) {
    return mock(true);
  }
  return invoke<boolean | null>('elevation_status');
}

/** 请求以管理员身份重启本程序，可带上待办角色 */
export function relaunchElevated(role: LinkRole | null): Promise<void> {
  if (MOCK) {
    return mock(undefined);
  }
  return invoke<void>('relaunch_elevated', { role });
}

/** 提权重启时带过来的待办角色 */
export function startupRole(): Promise<string | null> {
  if (MOCK) {
    return mock(null);
  }
  return invoke<string | null>('startup_role');
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
    return mock({ addr: '192.168.88.1', port: 27102, code: pairing, warning: null });
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
  platform: string | null,
): Promise<SendInfo> {
  if (MOCK) {
    void peer;
    void items;
    void pairing;
    void iface;
    void platform;
    return mock({ port: 27101, approved: true, dest: 'D:\\Games', message: '' }, 500);
  }
  return invoke<SendInfo>('start_send', { peer, items, pairing, iface, platform });
}

/** 列出本机所有 Steam 库，接收端挑装到哪个盘 */
export function steamLibraries(): Promise<SteamLibrary[]> {
  if (MOCK) {
    return mock([
      {
        label: 'D:\\SteamLibrary',
        install_dir: 'D:\\SteamLibrary\\steamapps\\common',
        claim_root: 'D:\\SteamLibrary\\steamapps',
        free_bytes: 240 * 1024 ** 3,
      },
      {
        label: 'C:\\Program Files (x86)\\Steam',
        install_dir: 'C:\\Program Files (x86)\\Steam\\steamapps\\common',
        claim_root: 'C:\\Program Files (x86)\\Steam\\steamapps',
        free_bytes: 42 * 1024 ** 3,
      },
    ]);
  }
  return invoke<SteamLibrary[]>('steam_libraries');
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
  claimRoot: string | null,
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
    claimRoot,
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
  from: '192.168.88.2',
  transfer_port: 27101,
  pairing: '424242',
  sender_name: 'BAIUPC',
  total_bytes: 64 * 1024 ** 3,
  items: [
    { name: '示例游戏 A', is_dir: true, bytes: 64 * 1024 ** 3 },
    { name: '存档备份', is_dir: true, bytes: 812 * 1024 ** 2 },
  ],
  want: '示例游戏 A',
  platform: 'steam',
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

/** 目录大小，挑选时按需算 */
export function dirSize(path: string): Promise<number> {
  if (MOCK) {
    return mock(64 * 1024 ** 2, 300);
  }
  return invoke<number>('dir_size', { path });
}

/** 订阅常驻发现推来的对端列表 */
export function onPeers(handler: (peers: PeerEntry[]) => void): Promise<UnlistenFn> {
  if (MOCK) {
    window.setTimeout(() => handler(mockPeers), 800);
    return Promise.resolve(() => undefined);
  }
  return listen<PeerEntry[]>('transfer://peers', (event) => {
    handler(event.payload);
  });
}

/** 订阅传入的传输请求 */
export function onRequest(handler: (payload: IncomingEvent) => void): Promise<UnlistenFn> {
  if (MOCK) {
    // 预览时用 window.__mockIncoming() 手动触发；地址栏带 ?incoming=1 打开会自动来一条
    (window as unknown as { __mockIncoming?: () => void }).__mockIncoming = () =>
      handler(mockRequest);
    const timer = window.location.search.includes('incoming=1')
      ? window.setTimeout(() => handler(mockRequest), 2500)
      : null;
    return Promise.resolve(() => {
      if (timer !== null) {
        window.clearTimeout(timer);
      }
    });
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
