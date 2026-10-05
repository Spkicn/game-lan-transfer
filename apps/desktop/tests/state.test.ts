import { describe, expect, it } from 'vitest';

import {
  canContinue,
  initialState,
  reduce,
  selectedGame,
  selectedPeer,
  type GameEntry,
  type PeerEntry,
} from '../src/state';

const game: GameEntry = {
  id: 'steam:D:/games/demo',
  name: '示例游戏',
  platform: 'steam',
  install_dir: 'D:/games/demo',
  size_bytes: 1024,
  fingerprint: 'build-1',
};

const peer: PeerEntry = {
  name: 'other',
  addr: '192.168.88.2',
  session_port: 27101,
  platform: 'windows',
  free_bytes: 2048,
};

describe('三屏状态机', () => {
  it('初始停在扫描屏', () => {
    expect(initialState.screen).toBe('scan');
    expect(canContinue(initialState)).toBe(false);
  });

  it('选中游戏与对端后可以进入下一步', () => {
    let state = reduce(initialState, { type: 'games', games: [game] });
    state = reduce(state, { type: 'select-game', id: game.id });
    state = reduce(state, { type: 'peers', peers: [peer] });
    state = reduce(state, { type: 'select-peer', addr: peer.addr });
    expect(canContinue(state)).toBe(true);
    expect(selectedGame(state)?.name).toBe('示例游戏');
    expect(selectedPeer(state)?.addr).toBe('192.168.88.2');
  });

  it('接收角色必须填写对端游戏标识', () => {
    let state = reduce(initialState, { type: 'peers', peers: [peer] });
    state = reduce(state, { type: 'select-peer', addr: peer.addr });
    state = reduce(state, { type: 'role', role: 'receive' });
    expect(canContinue(state)).toBe(false);
    state = reduce(state, { type: 'want', value: '123456' });
    expect(canContinue(state)).toBe(true);
  });

  it('改目标目录会清掉旧预检', () => {
    let state = reduce(initialState, {
      type: 'preview',
      preview: {
        title: '示例游戏',
        root_name: 'demo',
        total_bytes: 1024,
        file_count: 2,
        free_bytes: 4096,
        need_bytes: 2048,
        enough: true,
        has_old_copy: false,
        advice: '',
      },
    });
    state = reduce(state, { type: 'dest', value: 'E:/games' });
    expect(state.preview).toBeNull();
    expect(state.dest).toBe('E:/games');
  });

  it('出错会结束忙碌状态', () => {
    let state = reduce(initialState, { type: 'busy', busy: true });
    state = reduce(state, { type: 'error', message: '连接失败' });
    expect(state.busy).toBe(false);
    expect(state.error).toBe('连接失败');
  });

  it('复位保留目标目录', () => {
    let state = reduce(initialState, { type: 'dest', value: 'E:/games' });
    state = reduce(state, { type: 'screen', screen: 'transfer' });
    state = reduce(state, { type: 'reset' });
    expect(state.screen).toBe('scan');
    expect(state.dest).toBe('E:/games');
  });
});
