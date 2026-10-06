// 三阶段状态机：连接 → 选择内容 → 传输
//
// 只保存界面状态，不做任何 IO；所有数据都来自 Rust 侧命令

/** 三个阶段 */
export type Stage = 'connect' | 'pick' | 'transfer';

/** 内容来源 */
export type PickSource = 'games' | 'files';

/** 本机游戏 */
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

/** 本机目录条目 */
export interface LocalEntry {
  name: string;
  path: string;
  is_dir: boolean;
  bytes: number;
}

/** 网卡与地址 */
export interface NicEntry {
  name: string;
  description: string;
  is_physical: boolean;
  ip: string | null;
}

/** 网络现状 */
export interface NetworkStatus {
  nics: NicEntry[];
  address: string | null;
  nic_name: string | null;
  is_physical: boolean;
  direct_link: boolean;
  /** 本机在直连里的角色，未配置直连时为空 */
  link_role: LinkRole | null;
  /** 直连地址当前不可用时的原因 */
  address_problem: string | null;
  hint: string;
}

/** 直连角色：发送端用 .1，接收端用 .2 */
export type LinkRole = 'sender' | 'receiver';

/** 一次角色配置的结果 */
export interface RoleSetup {
  nic_name: string;
  ip: string;
  already: boolean;
}

/** 接收监听 */
export interface ListenInfo {
  addr: string;
  port: number;
  code: string | null;
  /** 请求端口不可用时的原因，此时对端能看到本机但发不来请求 */
  warning: string | null;
}

/** 传入请求里的一条内容 */
export interface IncomingItem {
  name: string;
  is_dir: boolean;
  bytes: number;
}

/** 传入请求 */
export interface IncomingEvent {
  id: number;
  /** 发送方地址，不含端口 */
  from: string;
  /** 发送方传输服务端口 */
  transfer_port: number;
  /** 发送方这次会话用的配对码 */
  pairing: string | null;
  sender_name: string;
  total_bytes: number;
  items: IncomingItem[];
  want: string;
  /** 内容所属平台，用来决定认领文件写到哪 */
  platform: string | null;
}

/** 发起结果 */
export interface SendInfo {
  port: number;
  approved: boolean;
  dest: string | null;
  message: string;
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

/** 这一单里本机是发送方还是接收方 */
export type TransferRole = 'send' | 'receive';

/** 队列里的一件内容 */
export interface TransferItem {
  name: string;
  bytes: number;
  /** 本机这一侧的来源路径，接收方没有 */
  source?: string;
}

/** 接收端可选的 Steam 库 */
export interface SteamLibrary {
  label: string;
  install_dir: string;
  claim_root: string;
  free_bytes: number;
}

/** 接收端在选择落盘位置 */
export interface DestPick {
  /** 选中的目标目录 */
  dir: string;
  /** 有 Steam 库时，认领文件要写到库目录而不是默认位置 */
  claimRoot: string | null;
}

/** 界面状态 */
export interface AppState {
  stage: Stage;
  busy: boolean;
  error: string | null;
  notice: string | null;
  network: NetworkStatus | null;
  /** 是否以管理员身份运行，配置直连需要 */
  elevated: boolean | null;
  /** 程序版本，诊断信息里带上 */
  version: string | null;
  /** 提权重启前选好的角色，重启后自动配好 */
  pendingRole: LinkRole | null;
  peers: PeerEntry[];
  peer: PeerEntry | null;
  pairing: string;
  listening: ListenInfo | null;
  incoming: IncomingEvent | null;
  incomingDest: string;
  /** 本机上的 Steam 库，接收端挑盘时用 */
  steamLibraries: SteamLibrary[];
  /** 接收端为目标目录选定的认领根，Steam 库时不是空 */
  destClaimRoot: string | null;
  /** 接收端浏览落盘位置时的当前目录 */
  destCwd: string | null;
  /** 接收端浏览落盘位置时的目录内容 */
  destEntries: LocalEntry[];
  games: GameEntry[];
  source: PickSource;
  picked: string[];
  cwd: string | null;
  entries: LocalEntry[];
  running: boolean;
  progress: Progress | null;
  sentBytes: number;
  sendResult: SendInfo | null;
  finished: RecvSummary | null;
  role: TransferRole | null;
  transferItems: TransferItem[];
}

/** 初始状态 */
export const initialState: AppState = {
  stage: 'connect',
  busy: false,
  error: null,
  notice: null,
  network: null,
  elevated: null,
  version: null,
  pendingRole: null,
  peers: [],
  peer: null,
  pairing: '',
  listening: null,
  incoming: null,
  incomingDest: '',
  steamLibraries: [],
  destClaimRoot: null,
  destCwd: null,
  destEntries: [],
  games: [],
  source: 'games',
  picked: [],
  cwd: null,
  entries: [],
  running: false,
  progress: null,
  sentBytes: 0,
  sendResult: null,
  finished: null,
  role: null,
  transferItems: [],
};

/** 状态变更 */
export type Action =
  | { type: 'stage'; stage: Stage }
  | { type: 'busy'; busy: boolean }
  | { type: 'error'; message: string | null }
  | { type: 'notice'; message: string | null }
  | { type: 'network'; network: NetworkStatus | null }
  | { type: 'elevated'; value: boolean | null }
  | { type: 'version'; value: string }
  | { type: 'version'; value: string }
  | { type: 'pending-role'; role: LinkRole | null }
  | { type: 'peers'; peers: PeerEntry[] }
  | { type: 'peer'; peer: PeerEntry | null }
  | { type: 'pairing'; value: string }
  | { type: 'listening'; listening: ListenInfo | null }
  | { type: 'incoming'; incoming: IncomingEvent | null }
  | { type: 'incoming-dest'; value: string }
  | { type: 'steam-libraries'; libraries: SteamLibrary[] }
  | { type: 'dest-claim-root'; value: string | null }
  | { type: 'dest-cwd'; value: string | null }
  | { type: 'dest-entries'; entries: LocalEntry[] }
  | { type: 'steam-libraries'; libraries: SteamLibrary[] }
  | { type: 'dest-claim-root'; value: string | null }
  | { type: 'dest-cwd'; value: string | null }
  | { type: 'dest-entries'; entries: LocalEntry[] }
  | { type: 'games'; games: GameEntry[] }
  | { type: 'source'; source: PickSource }
  | { type: 'toggle-pick'; path: string }
  | { type: 'clear-picks' }
  | { type: 'cwd'; cwd: string | null }
  | { type: 'entries'; entries: LocalEntry[] }
  | { type: 'running'; running: boolean }
  | { type: 'progress'; progress: Progress | null }
  | { type: 'sent'; bytes: number }
  | { type: 'send-result'; result: SendInfo | null }
  | { type: 'finished'; summary: RecvSummary | null }
  | { type: 'transfer-plan'; role: TransferRole; items: TransferItem[] }
  | { type: 'reset' };

/** 纯函数状态机 */
export function reduce(state: AppState, action: Action): AppState {
  switch (action.type) {
    case 'stage':
      return { ...state, stage: action.stage, error: null, notice: null };
    case 'busy':
      return { ...state, busy: action.busy };
    case 'error':
      // 出错就结束这一单的进行中状态，否则界面会一直停在传输中
      return { ...state, error: action.message, busy: false, running: false };
    case 'notice':
      return { ...state, notice: action.message };
    case 'network':
      return { ...state, network: action.network, busy: false };
    case 'elevated':
      return { ...state, elevated: action.value };
    case 'version':
      return { ...state, version: action.value };
    case 'pending-role':
      return { ...state, pendingRole: action.role };
    case 'peers':
      return { ...state, peers: action.peers, busy: false };
    case 'peer':
      return { ...state, peer: action.peer };
    case 'pairing':
      return { ...state, pairing: action.value };
    case 'listening':
      return { ...state, listening: action.listening, busy: false };
    case 'incoming':
      return { ...state, incoming: action.incoming };
    case 'incoming-dest':
      return { ...state, incomingDest: action.value };
    case 'steam-libraries':
      return { ...state, steamLibraries: action.libraries };
    case 'dest-claim-root':
      return { ...state, destClaimRoot: action.value };
    case 'dest-cwd':
      return { ...state, destCwd: action.value, busy: false };
    case 'dest-entries':
      return { ...state, destEntries: action.entries, busy: false };
    case 'games':
      return { ...state, games: action.games, busy: false };
    case 'source':
      return { ...state, source: action.source };
    case 'toggle-pick': {
      const picked = state.picked.includes(action.path)
        ? state.picked.filter((item) => item !== action.path)
        : [...state.picked, action.path];
      return { ...state, picked };
    }
    case 'clear-picks':
      return { ...state, picked: [] };
    case 'cwd':
      return { ...state, cwd: action.cwd, busy: false };
    case 'entries':
      return { ...state, entries: action.entries, busy: false };
    case 'running':
      return { ...state, running: action.running };
    case 'progress':
      return { ...state, progress: action.progress };
    case 'sent':
      return { ...state, sentBytes: action.bytes };
    case 'send-result':
      return {
        ...state,
        sendResult: action.result,
        // 对端同意后传输还在继续，只有被拒才算这一单结束
        running: action.result?.approved === true,
      };
    case 'finished':
      return { ...state, finished: action.summary, running: false };
    case 'transfer-plan':
      return { ...state, role: action.role, transferItems: action.items };
    case 'reset':
      return {
        ...initialState,
        network: state.network,
        pairing: state.pairing,
        incomingDest: state.incomingDest,
      };
  }
}

/** 已选内容的总字节数 */
export function pickedBytes(state: AppState): number {
  return state.picked.reduce((total, path) => {
    const game = state.games.find((entry) => entry.install_dir === path);
    if (game) {
      return total + game.size_bytes;
    }
    const local = state.entries.find((entry) => entry.path === path);
    return total + (local?.bytes ?? 0);
  }, 0);
}

/** 是否已经接上对端 */
export function connected(state: AppState): boolean {
  return state.peer !== null;
}

/** 本阶段的唯一动作是否可以按下 */
export function canAdvance(state: AppState): boolean {
  if (state.stage === 'connect') {
    return connected(state);
  }
  if (state.stage === 'pick') {
    return state.picked.length > 0;
  }
  return false;
}

/** 正在等待对端回应的一笔请求 */
export function awaitingIncoming(state: AppState): boolean {
  return state.incoming !== null;
}

/** 已选内容整理成队列条目 */
export function pickedItems(state: AppState): TransferItem[] {  return state.picked.map((path) => {
    const game = state.games.find((entry) => entry.install_dir === path);
    const local = state.entries.find((entry) => entry.path === path);
    return {
      name: path.split(/[\\/]/).pop() ?? path,
      bytes: game?.size_bytes ?? local?.bytes ?? 0,
      source: path,
    };
  });
}

/** 这一单是否已经搬完，接收方看结果，发送方看进度 */
export function transferComplete(state: AppState): boolean {
  if (state.finished) {
    return true;
  }
  // 没有这一单就没有完成一说，避免缺省进度把界面说成已完成
  if (state.role === null) {
    return false;
  }
  const progress = state.progress;
  return progress !== null && progress.bytes_total > 0 && progress.bytes_done >= progress.bytes_total;
}

/** 接收方拉取时该连的地址：发送方地址加传输端口
 *
 * 请求连接来自对端的临时端口，不能拿它去拉取
 */
export function pullAddress(incoming: IncomingEvent): string {
  return `${incoming.from}:${incoming.transfer_port}`;
}

/** 接收方拉取需要的东西：发送方传输地址、这次会话的配对码、平台
 *
 * 配对码用请求里带过来的那个，本机设置里填的不是发送方要的
 */
export interface PullPlan {
  peer: string;
  pairing: string | null;
  platform: string | null;
}

/** 组装接收方拉取所需参数 */
export function pullPlan(incoming: IncomingEvent): PullPlan {
  return {
    peer: pullAddress(incoming),
    pairing: incoming.pairing,
    platform: incoming.platform,
  };
}

/** 一份可复制的诊断文本，出问题时直接发给开发者 */
export function diagnosticsText(state: AppState): string {
  const network = state.network;
  const lines = [
    'GameLift 诊断',
    `版本: ${state.version ?? '未知'}`,
    `时间: ${new Date().toISOString()}`,
    `本机地址: ${network?.address ?? '无'}${network?.nic_name ? ` (${network.nic_name})` : ''}`,
    `角色: ${state.network?.link_role === 'sender' ? '发送端' : state.network?.link_role === 'receiver' ? '接收端' : '未配置'}`,
    `地址问题: ${network?.address_problem ?? '无'}`,
    `对端: ${state.peer ? `${state.peer.name} ${state.peer.addr}:${state.peer.session_port}` : '未接上'}`,
    `已选: ${state.picked.length} 项`,
    `本机 Steam 库: ${state.steamLibraries.length} 个`,
    `最近错误: ${state.error ?? '无'}`,
    `最近提示: ${state.notice ?? '无'}`,
  ];
  return lines.join('\n');
}

/** 全选的都是同一个平台的游戏时才带上平台，用来决定认领文件写到哪 */
export function pickedPlatform(state: AppState): string | null {
  const platforms = state.picked.map(
    (path) => state.games.find((game) => game.install_dir === path)?.platform ?? null,
  );
  const [first] = platforms;
  if (platforms.length === 0 || first === null || first === undefined) {
    return null;
  }
  return platforms.every((value) => value === first) ? first : null;
}

/** 队列里每一行此刻的状态 */export function rowState(state: AppState): 'waiting' | 'flowing' | 'done' {
  if (transferComplete(state)) {
    return 'done';
  }
  return state.running ? 'flowing' : 'waiting';
}
