// 开发预览用的假数据：仅当 VITE_MOCK=1 时启用，生产构建里会被摇掉
//
// 浏览器里没有 Tauri 运行时，用它把三阶段界面单独跑起来做视觉检查

import type {
  GameEntry,
  LocalEntry,
  NetworkStatus,
  PeerEntry,
  Progress,
} from './state';

/** 假网络 */
export const mockNetwork: NetworkStatus = {
  nics: [
    {
      name: '以太网',
      description: 'Gigabit Ethernet Controller',
      is_physical: true,
      ip: '192.168.88.1',
    },
    { name: 'WLAN', description: 'Wi-Fi 6 AX201', is_physical: false, ip: null },
  ],
  address: null,
  nic_name: '以太网',
  is_physical: true,
  direct_link: false,
  link_role: null,
  address_problem: null,
  hint: '两台机器直连时，选这台是发送端还是接收端，软件会自动配好地址',
};

/** 假对端 */
export const mockPeers: PeerEntry[] = [
  {
    name: 'BAIUPC',
    addr: '192.168.88.2',
    session_port: 27101,
    platform: 'windows',
    free_bytes: 240 * 1024 ** 3,
  },
];

/** 假游戏库 */
export const mockGames: GameEntry[] = [
  {
    id: 'mock:a',
    name: '示例游戏 A',
    platform: 'steam',
    install_dir: 'D:/SteamLibrary/steamapps/common/example_a',
    size_bytes: 64 * 1024 ** 3,
    fingerprint: '1000000',
  },
  {
    id: 'mock:b',
    name: '示例游戏 B',
    platform: 'epic',
    install_dir: 'D:/Epic/example_b',
    size_bytes: 12 * 1024 ** 3,
    fingerprint: 'build-42',
  },
];

/** 假目录 */
export const mockEntries: LocalEntry[] = [
  { name: '存档备份', path: 'D:/saves', is_dir: true, bytes: 0 },
  { name: '模组', path: 'D:/mods', is_dir: true, bytes: 0 },
  { name: '截图合集.zip', path: 'D:/shots.zip', is_dir: false, bytes: 812 * 1024 ** 2 },
];

/** 假进度序列 */
export function mockProgressStep(index: number): Progress {
  const total = 64 * 1024 ** 3;
  const done = Math.min(total, Math.round(total * (index / 20)));
  return {
    bytes_done: done,
    bytes_total: total,
    bytes_per_sec: 108 * 1024 ** 2,
    fraction: done / total,
  };
}
