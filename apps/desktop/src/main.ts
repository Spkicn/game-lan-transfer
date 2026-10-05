// 三屏界面：扫描 → 选择与预检 → 传输
//
// 界面只做渲染与状态推进，文件系统与网络全部交给 Rust 侧命令

import type { UnlistenFn } from '@tauri-apps/api/event';

import {
  cancelRecv,
  defaultDest,
  describeError,
  discoverPeers,
  onProgress,
  previewTarget,
  scanGames,
  startHost,
  startRecv,
  stopHost,
} from './api';
import { formatBytes, formatEta, formatRate, percent } from './format';
import {
  canContinue,
  initialState,
  reduce,
  selectedGame,
  selectedPeer,
  type AppState,
  type Progress,
} from './state';

const mountElement = document.getElementById('app');
if (!(mountElement instanceof HTMLElement)) {
  throw new Error('页面缺少挂载点 #app');
}
const mount: HTMLElement = mountElement;

let state: AppState = { ...initialState };
let stopProgress: UnlistenFn | null = null;

/** 转义文本，避免把用户数据当 HTML 解释 */
function text(value: string): string {
  const table: Record<string, string> = {
    '&': '&amp;',
    '<': '&lt;',
    '>': '&gt;',
    '"': '&quot;',
    "'": '&#39;',
  };
  return value.replace(/[&<>"']/g, (char) => table[char] ?? char);
}

/** 步骤指示器 */
function steps(active: number): string {
  const labels = ['扫描', '确认', '传输'];
  return `<ol class="steps">${labels
    .map(
      (label, index) =>
        `<li class="${index === active ? 'on' : ''}"><span>${index + 1}</span>${label}</li>`,
    )
    .join('')}</ol>`;
}

/** 顶部提示与错误 */
function banners(): string {
  const error = state.error ? `<p class="banner error">${text(state.error)}</p>` : '';
  const notice = state.notice ? `<p class="banner notice">${text(state.notice)}</p>` : '';
  return error + notice;
}

function scanView(): string {
  const games =
    state.games.length === 0
      ? '<p class="hint">还没有扫描，点上面的按钮</p>'
      : `<ul class="list">${state.games
          .map(
            (game) => `<li class="row ${game.id === state.selectedGameId ? 'on' : ''}"
              data-action="pick-game" data-id="${text(game.id)}">
              <span class="name">${text(game.name)}</span>
              <span class="meta">${text(game.platform)} · ${formatBytes(game.size_bytes)}</span>
            </li>`,
          )
          .join('')}</ul>`;
  const peers =
    state.peers.length === 0
      ? '<p class="hint">还没有发现对端，确保两台机器在同一网段</p>'
      : `<ul class="list">${state.peers
          .map((peer) => {
            const where =
              peer.session_port === 0
                ? `${text(peer.addr)}（仅接收）`
                : `${text(peer.addr)}:${peer.session_port}`;
            return `<li class="row ${peer.addr === state.selectedPeer ? 'on' : ''}"
              data-action="pick-peer" data-id="${text(peer.addr)}">
              <span class="name">${text(peer.name)}</span>
              <span class="meta">${where} · ${text(peer.platform)} · 可用 ${formatBytes(peer.free_bytes)}</span>
            </li>`;
          })
          .join('')}</ul>`;

  return `${steps(0)}
    ${banners()}
    <section class="card">
      <header>
        <h2>本机游戏</h2>
        <button data-action="scan" ${state.busy ? 'disabled' : ''}>扫描游戏库</button>
      </header>
      ${games}
    </section>
    <section class="card">
      <header>
        <h2>局域网对端</h2>
        <button data-action="discover" ${state.busy ? 'disabled' : ''}>发现对端</button>
      </header>
      ${peers}
    </section>
    <footer class="actions">
      <button class="primary" data-action="next" ${canContinue(state) ? '' : 'disabled'}>
        下一步
      </button>
    </footer>`;
}

function selectView(): string {
  const game = selectedGame(state);
  const peer = selectedPeer(state);
  const role =
    state.role === 'send'
      ? `<p class="hint">把本机选中的游戏发给对端：对端用「从对端接收」并输入配对码。</p>`
      : `<p class="hint">从对端拉取：填写对端上的游戏标识（命令行 <code>gamelift scan</code> 里的 appid 或目录名）。</p>`;
  const want =
    state.role === 'receive'
      ? `<label class="field"><span>对端游戏标识</span>
          <input id="want" value="${text(state.want)}" placeholder="例如 123456 或 example_game" />
        </label>`
      : '';
  const dest =
    state.role === 'receive'
      ? `<label class="field"><span>目标目录</span>
          <input id="dest" value="${text(state.dest)}" placeholder="例如 D:\\SteamLibrary\\steamapps\\common" />
        </label>`
      : '';
  const preview = state.preview;

  return `${steps(1)}
    ${banners()}
    <section class="card">
      <h2>本次搬运</h2>
      <div class="choices">
        <button class="${state.role === 'send' ? 'on' : ''}" data-action="role-send">发送本机游戏</button>
        <button class="${state.role === 'receive' ? 'on' : ''}" data-action="role-receive">从对端接收</button>
      </div>
      <dl class="summary">
        <dt>游戏</dt><dd>${game ? `${text(game.name)} · ${formatBytes(game.size_bytes)}` : '未选择'}</dd>
        <dt>对端</dt><dd>${peer ? `${text(peer.name)} · ${text(peer.addr)}` : '未选择'}</dd>
      </dl>
      ${role}
      ${want}
      ${dest}
      ${
        preview
          ? `<dl class="summary">
              <dt>内容</dt><dd>${text(preview.title)}（${preview.file_count} 个文件，${formatBytes(preview.total_bytes)}）</dd>
              <dt>目标盘</dt><dd>剩余 ${formatBytes(preview.free_bytes)}，需要 ${formatBytes(preview.need_bytes)}</dd>
              <dt>方式</dt><dd>${preview.has_old_copy ? '检测到已有副本，只传变化的块' : '首次传输，全量发送'}</dd>
              <dt>结论</dt><dd class="${preview.enough ? 'ok' : 'bad'}">${text(preview.advice)}</dd>
            </dl>`
          : ''
      }
    </section>
    <footer class="actions">
      <button data-action="back">返回</button>
      ${
        state.role === 'receive'
          ? `<button data-action="check" ${state.busy ? 'disabled' : ''}>检查目标盘</button>
             <button class="primary" data-action="start-receive" ${
               state.preview?.enough === false ? 'disabled' : ''
             }>开始接收</button>`
          : `<button class="primary" data-action="start-send" ${state.busy ? 'disabled' : ''}>开始等待对端</button>`
      }
    </footer>`;
}

function transferView(): string {
  if (state.host) {
    return `${steps(2)}
      ${banners()}
      <section class="card">
        <h2>等待对端接收</h2>
        <p class="code">配对码 <strong>${text(state.host.code)}</strong></p>
        <p>在另一台机器上打开 GameLift，选择「从对端接收」，地址填
          <code>${text(state.host.addr)}:${String(state.host.port)}</code>，配对码填上面的数字。</p>
        <p class="hint">内容：${text(state.host.title)}</p>
      </section>
      <footer class="actions">
        <button data-action="stop-host">停止共享</button>
      </footer>`;
  }

  const progress = state.progress;
  const ratio = progress ? percent(progress.bytes_done, progress.bytes_total) : 0;
  const summary = state.summary;
  const bar = summary
    ? `<p class="ok">已完成：${formatBytes(summary.bytes_total)} → <code>${text(summary.root)}</code></p>
       ${summary.bytes_resumed > 0 ? `<p class="hint">其中 ${formatBytes(summary.bytes_resumed)} 由续传或差异跳过</p>` : ''}
       ${
         summary.claim_files.length > 0
           ? `<p class="hint">已写入认领文件：${summary.claim_files.map((file) => text(file)).join('、')}</p>`
           : '<p class="hint">没有认领文件，按启动器的「验证文件完整性」收尾即可</p>'
       }`
    : `<div class="bar"><div id="progress-fill" style="width: ${ratio.toFixed(1)}%"></div></div>
       <p id="progress-text" class="hint">${
         progress
           ? `${ratio.toFixed(1)}%  ${formatBytes(progress.bytes_done)} / ${formatBytes(progress.bytes_total)}`
           : '正在连接…'
       }</p>`;

  return `${steps(2)}
    ${banners()}
    <section class="card">
      <h2>${summary ? '传输完成' : '正在传输'}</h2>
      ${bar}
    </section>
    <footer class="actions">
      ${
        summary
          ? '<button class="primary" data-action="again">再搬一次</button>'
          : '<button data-action="cancel">取消</button>'
      }
    </footer>`;
}

/** 渲染当前界面 */
function render(): void {
  let view: string;
  switch (state.screen) {
    case 'scan':
      view = scanView();
      break;
    case 'select':
      view = selectView();
      break;
    case 'transfer':
      view = transferView();
      break;
  }
  mount.innerHTML = view;
}

/** 只刷新进度条，避免整屏重绘 */
function refreshProgress(): void {
  const progress = state.progress;
  const fill = document.getElementById('progress-fill');
  const label = document.getElementById('progress-text');
  if (!progress || !fill || !label) {
    return;
  }
  const ratio = percent(progress.bytes_done, progress.bytes_total);
  fill.style.width = `${ratio.toFixed(1)}%`;
  label.textContent = `${ratio.toFixed(1)}%  ${formatBytes(progress.bytes_done)} / ${formatBytes(
    progress.bytes_total,
  )}  速度 ${formatRate(progress.bytes_per_sec)}  剩余 ${formatEta(
    progress.bytes_done,
    progress.bytes_total,
    progress.bytes_per_sec,
  )}`;
}

/** 预检目标盘 */
async function runPreview(): Promise<void> {
  const peer = selectedPeer(state);
  if (!peer) {
    throw new Error('请先在第一步选择对端');
  }
  if (state.dest.trim().length === 0) {
    throw new Error('请填写目标目录');
  }
  state = reduce(state, { type: 'busy', busy: true });
  render();
  const preview = await previewTarget(peer.addr, state.want.trim(), state.dest, null);
  state = reduce(state, { type: 'preview', preview });
  state = reduce(state, { type: 'busy', busy: false });
}

/** 作为源端开始等待 */
async function runSend(): Promise<void> {
  const game = selectedGame(state);
  if (!game) {
    throw new Error('请先在第一步选择要发送的游戏');
  }
  state = reduce(state, { type: 'busy', busy: true });
  render();
  const info = await startHost(game.install_dir, game.name, game.platform, null);
  state = reduce(state, { type: 'host', host: info });
  state = reduce(state, { type: 'busy', busy: false });
  state = reduce(state, { type: 'screen', screen: 'transfer' });
}

/** 作为目标端开始接收 */
async function runReceive(): Promise<void> {
  const peer = selectedPeer(state);
  if (!peer) {
    throw new Error('请先在第一步选择对端');
  }
  if (state.dest.trim().length === 0) {
    throw new Error('请填写目标目录');
  }
  state = reduce(state, { type: 'screen', screen: 'transfer' });
  render();
  stopProgress = await onProgress((progress: Progress) => {
    state = reduce(state, { type: 'progress', progress });
    refreshProgress();
  });
  try {
    const game = selectedGame(state);
    const summary = await startRecv(
      peer.addr,
      state.want.trim(),
      state.dest,
      game?.platform ?? null,
      null,
      true,
    );
    state = reduce(state, { type: 'summary', summary });
  } finally {
    if (stopProgress) {
      stopProgress();
      stopProgress = null;
    }
  }
}

/** 双击动作入口 */
async function handle(action: string, id: string | null): Promise<void> {
  try {
    switch (action) {
      case 'scan': {
        state = reduce(state, { type: 'busy', busy: true });
        render();
        state = reduce(state, { type: 'games', games: await scanGames() });
        break;
      }
      case 'discover': {
        state = reduce(state, { type: 'busy', busy: true });
        render();
        state = reduce(state, { type: 'peers', peers: await discoverPeers(3) });
        break;
      }
      case 'pick-game':
        if (id) {
          state = reduce(state, { type: 'select-game', id });
        }
        break;
      case 'pick-peer':
        if (id) {
          state = reduce(state, { type: 'select-peer', addr: id });
        }
        break;
      case 'role-send':
        state = reduce(state, { type: 'role', role: 'send' });
        break;
      case 'role-receive':
        state = reduce(state, { type: 'role', role: 'receive' });
        break;
      case 'next':
        state = reduce(state, { type: 'screen', screen: 'select' });
        break;
      case 'back':
        state = reduce(state, { type: 'screen', screen: 'scan' });
        break;
      case 'check':
        await runPreview();
        break;
      case 'start-send':
        await runSend();
        break;
      case 'start-receive':
        await runReceive();
        break;
      case 'cancel':
        await cancelRecv();
        state = reduce(state, {
          type: 'notice',
          message: '已请求取消，已完成的块会保留，重跑即可续传',
        });
        break;
      case 'stop-host':
        await stopHost();
        state = reduce(state, { type: 'host', host: null });
        break;
      case 'again':
        state = reduce(state, { type: 'reset' });
        break;
      default:
        break;
    }
  } catch (error) {
    state = reduce(state, { type: 'error', message: describeError(error) });
  }
  render();
}

mount.addEventListener('click', (event) => {
  const target = event.target;
  if (!(target instanceof HTMLElement)) {
    return;
  }
  const button = target.closest<HTMLElement>('[data-action]');
  const action = button?.dataset.action;
  if (!action) {
    return;
  }
  const id = target.closest<HTMLElement>('[data-id]')?.dataset.id ?? null;
  void handle(action, id);
});

mount.addEventListener('input', (event) => {
  const target = event.target;
  if (!(target instanceof HTMLInputElement)) {
    return;
  }
  if (target.id === 'dest') {
    state = reduce(state, { type: 'dest', value: target.value });
  }
  if (target.id === 'want') {
    state = reduce(state, { type: 'want', value: target.value });
  }
});

/** 启动：取一次默认目标目录 */
async function boot(): Promise<void> {
  try {
    const dest = await defaultDest();
    if (dest) {
      state = reduce(state, { type: 'dest', value: dest });
      state = reduce(state, { type: 'notice', message: `已选用本机 Steam 库：${dest}` });
    }
  } catch (error) {
    state = reduce(state, { type: 'error', message: describeError(error) });
  }
  render();
}

void boot();
