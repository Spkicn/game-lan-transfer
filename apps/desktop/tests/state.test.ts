import { describe, expect, it } from 'vitest';

import {
  canAdvance,
  connected,
  diagnosticsText,
  initialState,
  pickedBytes,
  pickedItems,
  pickedPlatform,
  pullPlan,
  reduce,
  rowState,
  transferComplete,
  type GameEntry,
  type PeerEntry,
} from '../src/lib/state';

const game: GameEntry = {
  id: 'steam:D:/games/demo',
  name: '示例游戏',
  platform: 'steam',
  install_dir: 'D:/games/demo',
  size_bytes: 4096,
  fingerprint: 'build-1',
};

const peer: PeerEntry = {
  name: 'other',
  addr: '192.168.88.2',
  session_port: 27101,
  platform: 'windows',
  free_bytes: 8192,
};

describe('三阶段状态机', () => {
  it('从连接阶段开始，未接上对端不能往下走', () => {
    expect(initialState.stage).toBe('connect');
    expect(connected(initialState)).toBe(false);
    expect(canAdvance(initialState)).toBe(false);
  });

  it('接上对端后可以进入内容阶段', () => {
    const state = reduce(initialState, { type: 'peer', peer });
    expect(connected(state)).toBe(true);
    expect(canAdvance(state)).toBe(true);
  });

  it('选中的项可以来回切换，清空后回到未满足', () => {
    let state = reduce(initialState, { type: 'source', source: 'games' });
    state = reduce(state, { type: 'games', games: [game] });
    state = reduce(state, { type: 'toggle-pick', path: game.install_dir });
    expect(state.picked).toEqual([game.install_dir]);
    expect(pickedBytes(state)).toBe(4096);
    state = reduce(state, { type: 'toggle-pick', path: game.install_dir });
    expect(state.picked).toEqual([]);
    state = reduce(state, { type: 'toggle-pick', path: game.install_dir });
    state = reduce(state, { type: 'clear-picks' });
    expect(state.picked).toEqual([]);
  });

  it('已选字节数会算上本机目录条目', () => {
    let state = reduce(initialState, {
      type: 'entries',
      entries: [{ name: 'a.bin', path: 'D:/a.bin', is_dir: false, bytes: 1024 }],
    });
    state = reduce(state, { type: 'toggle-pick', path: 'D:/a.bin' });
    expect(pickedBytes(state)).toBe(1024);
  });

  it('传入请求只在被处理时清空，目标目录独立保存', () => {
    let state = reduce(initialState, {
      type: 'incoming',
      incoming: {
        id: 7,
        from: '192.168.88.2',
        transfer_port: 27101,
        pairing: '424242',
        sender_name: 'laptop',
        total_bytes: 2048,
        items: [{ name: 'demo', is_dir: true, bytes: 2048 }],
        want: 'demo',
        platform: 'steam',
      },
    });
    state = reduce(state, { type: 'incoming-dest', value: 'D:/games' });
    expect(state.incoming?.id).toBe(7);
    expect(state.incomingDest).toBe('D:/games');
    state = reduce(state, { type: 'incoming', incoming: null });
    expect(state.incoming).toBeNull();
  });

  it('错误会结束忙碌与进行中状态并保留阶段', () => {
    let state = reduce(initialState, { type: 'busy', busy: true });
    state = reduce(state, { type: 'running', running: true });
    state = reduce(state, { type: 'error', message: '连接超时' });
    expect(state.busy).toBe(false);
    expect(state.running).toBe(false);
    expect(state.error).toBe('连接超时');
    expect(state.stage).toBe('connect');
  });

  it('复位回到连接阶段并保留网络与配对码', () => {
    let state = reduce(initialState, { type: 'pairing', value: '123456' });
    state = reduce(state, { type: 'stage', stage: 'transfer' });
    state = reduce(state, { type: 'reset' });
    expect(state.stage).toBe('connect');
    expect(state.pairing).toBe('123456');
  });

  it('队列条目从已选内容整理出来，名字取末段', () => {
    let state = reduce(initialState, { type: 'games', games: [game] });
    state = reduce(state, {
      type: 'entries',
      entries: [{ name: 'a.bin', path: 'D:/a.bin', is_dir: false, bytes: 1024 }],
    });
    state = reduce(state, { type: 'toggle-pick', path: game.install_dir });
    state = reduce(state, { type: 'toggle-pick', path: 'D:/a.bin' });
    expect(pickedItems(state)).toEqual([
      { name: 'demo', bytes: 4096, source: 'D:/games/demo' },
      { name: 'a.bin', bytes: 1024, source: 'D:/a.bin' },
    ]);
  });

  it('记录本机在这一单里的角色与队列', () => {
    let state = reduce(initialState, {
      type: 'transfer-plan',
      role: 'receive',
      items: [{ name: 'demo', bytes: 2048 }],
    });
    expect(state.role).toBe('receive');
    expect(state.transferItems).toEqual([{ name: 'demo', bytes: 2048 }]);
    expect(rowState(state)).toBe('waiting');
    state = reduce(state, { type: 'running', running: true });
    expect(rowState(state)).toBe('flowing');
    state = reduce(state, {
      type: 'finished',
      summary: {
        root: 'D:/games',
        bytes_received: 2048,
        bytes_total: 2048,
        bytes_resumed: 0,
        claim_files: [],
      },
    });
    expect(rowState(state)).toBe('done');
  });

  it('全选同一个平台的游戏才带平台', () => {
    let state = reduce(initialState, { type: 'games', games: [game] });
    expect(pickedPlatform(state)).toBeNull();
    state = reduce(state, { type: 'toggle-pick', path: game.install_dir });
    expect(pickedPlatform(state)).toBe('steam');
    // 混入一个文件夹就不再带平台
    state = reduce(state, {
      type: 'entries',
      entries: [{ name: 'a.bin', path: 'D:/a.bin', is_dir: false, bytes: 1024 }],
    });
    state = reduce(state, { type: 'toggle-pick', path: 'D:/a.bin' });
    expect(pickedPlatform(state)).toBeNull();
  });

  it('已完成的传输会留在列表里，重复添加只留一条', () => {
    const record = {
      key: 'receive:1:demo',
      name: 'demo',
      bytes: 4096,
      direction: 'receive' as const,
      dest: 'E:\\Steam\\steamapps\\common',
      at: 1,
    };
    let state = reduce(initialState, { type: 'history-add', records: [record] });
    expect(state.history).toHaveLength(1);
    state = reduce(state, { type: 'history-add', records: [record] });
    expect(state.history).toHaveLength(1);
    state = reduce(state, {
      type: 'history-add',
      records: [{ ...record, key: 'receive:1:other', name: 'other' }],
    });
    expect(state.history).toHaveLength(2);
    expect(state.history[0]?.name).toBe('other');
    state = reduce(state, { type: 'history-clear' });
    expect(state.history).toHaveLength(0);
  });

  it('诊断信息把当前状态一次说全', () => {
    let state = reduce(initialState, {
      type: 'network',
      network: {
        nics: [],
        address: '192.168.88.1',
        nic_name: '以太网',
        is_physical: true,
        direct_link: true,
        link_role: 'sender',
        address_problem: null,
        hint: '',
      },
    });
    state = reduce(state, { type: 'version', value: '0.2.3' });
    state = reduce(state, { type: 'peer', peer });
    state = reduce(state, { type: 'error', message: '连接超时' });
    const text = diagnosticsText(state);
    expect(text).toContain('版本: 0.2.3');
    expect(text).toContain('本机地址: 192.168.88.1 (以太网)');
    expect(text).toContain('角色: 发送端');
    expect(text).toContain('对端: other 192.168.88.2:27101');
    expect(text).toContain('最近错误: 连接超时');
  });

  it('拉取要用发送方的传输端口与配对码，不能用本机设置', () => {
    const plan = pullPlan({
      id: 1,
      from: '192.168.88.1',
      transfer_port: 27101,
      pairing: '424242',
      sender_name: 'laptop',
      total_bytes: 1024,
      items: [{ name: 'demo', is_dir: true, bytes: 1024 }],
      want: 'demo',
      platform: 'steam',
    });
    expect(plan).toEqual({
      peer: '192.168.88.1:27101',
      pairing: '424242',
      platform: 'steam',
    });
  });

  it('接收端挑落盘位置的状态各自独立', () => {
    let state = reduce(initialState, {
      type: 'steam-libraries',
      libraries: [
        {
          label: 'D:\\SteamLibrary',
          install_dir: 'D:\\SteamLibrary\\steamapps\\common',
          claim_root: 'D:\\SteamLibrary\\steamapps',
          free_bytes: 1024,
        },
      ],
    });
    expect(state.steamLibraries).toHaveLength(1);
    state = reduce(state, {
      type: 'incoming-dest',
      value: 'D:\\SteamLibrary\\steamapps\\common',
    });
    state = reduce(state, {
      type: 'dest-claim-root',
      value: 'D:\\SteamLibrary\\steamapps',
    });
    state = reduce(state, { type: 'dest-cwd', value: 'D:\\Games' });
    state = reduce(state, {
      type: 'dest-entries',
      entries: [{ name: 'saves', path: 'D:\\Games\\saves', is_dir: true, bytes: 0 }],
    });
    expect(state.destClaimRoot).toBe('D:\\SteamLibrary\\steamapps');
    expect(state.destCwd).toBe('D:\\Games');
    expect(state.destEntries).toHaveLength(1);
    expect(state.incomingDest).toBe('D:\\SteamLibrary\\steamapps\\common');
  });

  it('对端同意后传输仍在进行，被拒才算结束', () => {
    const approved = reduce(initialState, {
      type: 'send-result',
      result: { port: 27101, approved: true, dest: 'D:/Games', message: '' },
    });
    expect(approved.running).toBe(true);
    const rejected = reduce(initialState, {
      type: 'send-result',
      result: { port: 27101, approved: false, dest: null, message: '不同意' },
    });
    expect(rejected.running).toBe(false);
  });

  it('发送方按进度判断完成，不依赖接收结果', () => {
    let state = reduce(initialState, {
      type: 'transfer-plan',
      role: 'send',
      items: [{ name: 'demo', bytes: 4096 }],
    });
    state = reduce(state, { type: 'running', running: true });
    expect(transferComplete(state)).toBe(false);
    state = reduce(state, {
      type: 'progress',
      progress: { bytes_done: 1024, bytes_total: 4096, bytes_per_sec: 512, fraction: 0.25 },
    });
    expect(transferComplete(state)).toBe(false);
    expect(rowState(state)).toBe('flowing');
    state = reduce(state, {
      type: 'progress',
      progress: { bytes_done: 4096, bytes_total: 4096, bytes_per_sec: 512, fraction: 1 },
    });
    expect(transferComplete(state)).toBe(true);
    expect(rowState(state)).toBe('done');
  });

  it('还没开始这一单时，进度事件不会把界面说成已完成', () => {
    const state = reduce(initialState, {
      type: 'progress',
      progress: { bytes_done: 4096, bytes_total: 4096, bytes_per_sec: 512, fraction: 1 },
    });
    expect(state.role).toBeNull();
    expect(transferComplete(state)).toBe(false);
  });

  it('总量为零不当作完成', () => {
    let state = reduce(initialState, {
      type: 'transfer-plan',
      role: 'receive',
      items: [{ name: 'demo', bytes: 0 }],
    });
    state = reduce(state, {
      type: 'progress',
      progress: { bytes_done: 0, bytes_total: 0, bytes_per_sec: 0, fraction: 0 },
    });
    expect(transferComplete(state)).toBe(false);
  });
});
