// 三屏状态机：扫描 → 选择与预检 → 传输
//
// 只保存界面状态，不做任何 IO；所有数据都来自 Rust 侧命令

/** 三个界面 */
export type Screen = 'scan' | 'select' | 'transfer';

/** 本机在本次搬运中的角色 */
export type Role = 'send' | 'receive';

/** 扫描到的游戏 */
export interface GameEntry {
  id: string;
  name: string;
  platform: string;
  install_dir: string;
  size_bytes: number;
  fingerprint: string | null;
}

/** 发现到的对端 */
export interface PeerEntry {
  name: string;
  addr: string;
  session_port: number;
  platform: string;
  free_bytes: number;
}

/** 传输前预检 */
export interface Preview {
  title: string;
  root_name: string;
  total_bytes: number;
  file_count: number;
  free_bytes: number;
  need_bytes: number;
  enough: boolean;
  has_old_copy: boolean;
  advice: string;
}

/** 传输进度 */
export interface Progress {
  bytes_done: number;
  bytes_total: number;
  bytes_per_sec: number;
  fraction: number;
}

/** 接收结果 */
export interface RecvSummary {
  root: string;
  bytes_received: number;
  bytes_total: number;
  bytes_resumed: number;
  claim_files: string[];
}

/** 源端信息 */
export interface HostInfo {
  addr: string;
  port: number;
  code: string;
  title: string;
}

/** 界面状态 */
export interface AppState {
  screen: Screen;
  busy: boolean;
  games: GameEntry[];
  peers: PeerEntry[];
  selectedGameId: string | null;
  selectedPeer: string | null;
  role: Role;
  want: string;
  dest: string;
  preview: Preview | null;
  progress: Progress | null;
  host: HostInfo | null;
  summary: RecvSummary | null;
  error: string | null;
  notice: string | null;
}

/** 初始状态 */
export const initialState: AppState = {
  screen: 'scan',
  busy: false,
  games: [],
  peers: [],
  selectedGameId: null,
  selectedPeer: null,
  role: 'send',
  want: '',
  dest: '',
  preview: null,
  progress: null,
  host: null,
  summary: null,
  error: null,
  notice: null,
};

/** 状态变更 */
export type Action =
  | { type: 'games'; games: GameEntry[] }
  | { type: 'peers'; peers: PeerEntry[] }
  | { type: 'select-game'; id: string }
  | { type: 'select-peer'; addr: string }
  | { type: 'role'; role: Role }
  | { type: 'want'; value: string }
  | { type: 'dest'; value: string }
  | { type: 'preview'; preview: Preview | null }
  | { type: 'screen'; screen: Screen }
  | { type: 'busy'; busy: boolean }
  | { type: 'progress'; progress: Progress }
  | { type: 'host'; host: HostInfo | null }
  | { type: 'summary'; summary: RecvSummary | null }
  | { type: 'error'; message: string | null }
  | { type: 'notice'; message: string | null }
  | { type: 'reset' };

/** 纯函数状态机 */
export function reduce(state: AppState, action: Action): AppState {
  switch (action.type) {
    case 'games':
      return { ...state, games: action.games, busy: false, error: null };
    case 'peers':
      return { ...state, peers: action.peers, busy: false, error: null };
    case 'select-game':
      return { ...state, selectedGameId: action.id, preview: null };
    case 'select-peer':
      return { ...state, selectedPeer: action.addr, preview: null };
    case 'role':
      return { ...state, role: action.role, preview: null };
    case 'want':
      return { ...state, want: action.value };
    case 'dest':
      return { ...state, dest: action.value, preview: null };
    case 'preview':
      return { ...state, preview: action.preview };
    case 'screen':
      return { ...state, screen: action.screen, error: null };
    case 'busy':
      return { ...state, busy: action.busy };
    case 'progress':
      return { ...state, progress: action.progress };
    case 'host':
      return { ...state, host: action.host };
    case 'summary':
      return { ...state, summary: action.summary };
    case 'error':
      return { ...state, error: action.message, busy: false };
    case 'notice':
      return { ...state, notice: action.message };
    case 'reset':
      return { ...initialState, dest: state.dest };
  }
}

/** 选中的游戏 */
export function selectedGame(state: AppState): GameEntry | null {
  return state.games.find((game) => game.id === state.selectedGameId) ?? null;
}

/** 选中的对端 */
export function selectedPeer(state: AppState): PeerEntry | null {
  return state.peers.find((peer) => peer.addr === state.selectedPeer) ?? null;
}

/** 是否满足进入下一步的条件 */
export function canContinue(state: AppState): boolean {
  if (state.selectedPeer === null) {
    return false;
  }
  if (state.role === 'receive') {
    return state.want.trim().length > 0;
  }
  return selectedGame(state) !== null;
}

/** 传输是否已经开始并有进度可展示 */
export function hasProgress(state: AppState): boolean {
  return state.progress !== null;
}
