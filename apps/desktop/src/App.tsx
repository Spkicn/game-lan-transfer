// 三阶段单窗口：连接 → 选择内容 → 传输
//
// 界面只做渲染与状态推进，文件系统与网络全部交给 Rust 侧命令

import { useCallback, useEffect, useReducer } from 'react';
import {
  ArrowUp,
  Check,
  FolderUp,
  Gamepad2,
  HardDrive,
  Loader2,
  PlugZap,
  RefreshCw,
  X,
} from 'lucide-react';

import { Lamp, LinkRail, Port, RailRow, StatusStrip, type LampState } from './components/Panel';
import { Button } from './components/ui/button';
import { Input } from './components/ui/input';
import * as api from './lib/api';
import { formatBytes, formatEta, formatRate, percent } from './lib/format';
import {
  canAdvance,
  connected,
  initialState,
  pickedBytes,
  reduce,
  type AppState,
  type IncomingEvent,
  type LocalEntry,
} from './lib/state';

const STAGES = [
  { id: 'connect', label: '连接' },
  { id: 'pick', label: '内容' },
  { id: 'transfer', label: '传输' },
] as const;

/** 三阶段单窗口 */
export default function App() {
  const [state, dispatch] = useReducer(reduce, initialState);

  // 事件：进度与传入请求
  useEffect(() => {
    let alive = true;
    const offs: Array<() => void> = [];
    void api
      .onProgress((progress) => dispatch({ type: 'progress', progress }))
      .then((off) => (alive ? offs.push(off) : off()));
    void api
      .onRequest((incoming) => dispatch({ type: 'incoming', incoming }))
      .then((off) => (alive ? offs.push(off) : off()));
    return () => {
      alive = false;
      for (const off of offs) {
        off();
      }
    };
  }, []);

  // 启动：读网络状态
  useEffect(() => {
    void (async () => {
      try {
        dispatch({ type: 'network', network: await api.networkStatus() });
      } catch (error) {
        dispatch({ type: 'error', message: api.describeError(error) });
      }
    })();
  }, []);

  // 发送期间轮询源端已下发字节
  useEffect(() => {
    if (!state.running || state.stage !== 'transfer') {
      return;
    }
    const timer = window.setInterval(() => {
      void api
        .sendProgress()
        .then((bytes) => dispatch({ type: 'sent', bytes }))
        .catch(() => undefined);
    }, 700);
    return () => window.clearInterval(timer);
  }, [state.running, state.stage]);

  const run = useCallback(async (work: () => Promise<void>) => {
    dispatch({ type: 'busy', busy: true });
    try {
      await work();
    } catch (error) {
      dispatch({ type: 'error', message: api.describeError(error) });
    } finally {
      dispatch({ type: 'busy', busy: false });
    }
  }, []);

  const iface = state.network?.address ?? null;
  const pairing = state.pairing.trim().length > 0 ? state.pairing.trim() : null;

  return (
    <div className="grid h-full grid-rows-[auto_1fr_auto] bg-panel">
      <header className="flex items-stretch gap-6 border-b border-panel-edge bg-panel-rail px-5 pt-3">
        <span className="label mb-3 self-center">GameLift</span>
        <nav className="flex items-end gap-1" aria-label="阶段">
          {STAGES.map((stage, index) => {
            const active = state.stage === stage.id;
            const reachable =
              index === 0 || (index === 1 && connected(state)) || (index === 2 && state.running);
            return (
              <button
                key={stage.id}
                type="button"
                disabled={!reachable}
                onClick={() => dispatch({ type: 'stage', stage: stage.id })}
                className={[
                  'label border border-b-0 px-4 pt-2 pb-2.5 transition-colors duration-100',
                  active
                    ? 'border-panel-edge bg-panel text-ink'
                    : 'border-transparent text-ink-faint hover:text-ink-dim',
                  reachable ? '' : 'opacity-40',
                ].join(' ')}
              >
                {stage.label}
              </button>
            );
          })}
        </nav>
        <span className="readi ml-auto mb-3 self-center text-[12px] text-ink-faint">
          {state.network?.address ?? '无可用地址'}
        </span>
      </header>

      <main className="min-h-0 overflow-y-auto px-5 py-5">
        {state.stage === 'connect' ? <ConnectStage state={state} dispatch={dispatch} run={run} iface={iface} /> : null}
        {state.stage === 'pick' ? (
          <PickStage state={state} dispatch={dispatch} run={run} iface={iface} pairing={pairing} />
        ) : null}
        {state.stage === 'transfer' ? (
          <TransferStage state={state} dispatch={dispatch} run={run} iface={iface} pairing={pairing} />
        ) : null}
      </main>

      <StatusStrip>
        <NextAction state={state} dispatch={dispatch} run={run} iface={iface} pairing={pairing} />
      </StatusStrip>
    </div>
  );
}

type Dispatch = React.Dispatch<Parameters<typeof reduce>[1]>;
type Run = (work: () => Promise<void>) => Promise<void>;

/** 连接阶段：两个端口与一根跳线 */
function ConnectStage({
  state,
  dispatch,
  run,
  iface,
}: {
  state: AppState;
  dispatch: Dispatch;
  run: Run;
  iface: string | null;
}) {
  const network = state.network;
  const peer = state.peer;
  const linkState: LampState = peer ? 'ready' : state.busy ? 'flow' : 'idle';

  const discover = () =>
    run(async () => {
      const peers = await api.discoverPeers(iface, 3);
      dispatch({ type: 'peers', peers });
      if (peers.length > 0 && state.peer === null) {
        dispatch({ type: 'peer', peer: peers[0] ?? null });
      }
      const listening = await api.startListen(iface, state.pairing.trim() || null);
      dispatch({ type: 'listening', listening });
      if (peers.length === 0) {
        dispatch({ type: 'notice', message: '没有发现对端，确认两台机器在同一网段，或让对方也打开本软件' });
      }
    });

  return (
    <div className="grid gap-6 lg:grid-cols-[1fr_auto_1fr]">
      <Port
        label="本机"
        name={network?.nic_name ?? '未检测到网卡'}
        endpoint={network?.address ?? '—'}
        lamp={network?.address ? 'ready' : 'fault'}
      >
        <div className="flex flex-wrap gap-2">
          <Button size="sm" onClick={() => void run(async () => dispatch({ type: 'network', network: await api.networkStatus() }))}>
            <RefreshCw className="size-3.5" /> 重新检测
          </Button>
          <Button
            size="sm"
            onClick={() =>
              void run(async () => {
                const address = await api.setupLink(1);
                dispatch({ type: 'notice', message: `已配置直连地址 ${address}` });
                dispatch({ type: 'network', network: await api.networkStatus() });
              })
            }
          >
            <PlugZap className="size-3.5" /> 配置直连 .1
          </Button>
          <Button
            size="sm"
            onClick={() =>
              void run(async () => {
                const address = await api.setupLink(2);
                dispatch({ type: 'notice', message: `已配置直连地址 ${address}` });
                dispatch({ type: 'network', network: await api.networkStatus() });
              })
            }
          >
            配置直连 .2
          </Button>
          <Button
            size="sm"
            variant="quiet"
            onClick={() =>
              void run(async () => {
                await api.revertLink();
                dispatch({ type: 'notice', message: '直连配置已还原' });
                dispatch({ type: 'network', network: await api.networkStatus() });
              })
            }
          >
            还原
          </Button>
        </div>
        <p className="text-[12px] text-ink-faint">{network?.hint ?? '正在读取网络'}</p>
      </Port>

      <LinkRail
        state={linkState}
        caption={peer ? '跳线已接上' : state.busy ? '正在找对端' : '等待跳线'}
      />

      <Port
        label="对端"
        name={peer?.name ?? '空插槽'}
        endpoint={peer ? `${peer.addr}:${peer.session_port}` : '未接上'}
        lamp={peer ? 'ready' : 'idle'}
        slot={peer === null}
      >
        {peer ? (
          <div className="flex flex-wrap gap-2">
            <Button size="sm" onClick={() => dispatch({ type: 'stage', stage: 'pick' })}>
              <Check className="size-3.5" /> 接上，去选内容
            </Button>
            <Button size="sm" variant="quiet" onClick={() => dispatch({ type: 'peer', peer: null })}>
              断开
            </Button>
          </div>
        ) : (
          <div className="flex flex-wrap gap-2">
            <Button size="sm" onClick={() => void discover()} disabled={state.busy}>
              {state.busy ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
              找对端
            </Button>
          </div>
        )}
        {state.peers.length > 1 ? (
          <ul className="border border-panel-edge">
            {state.peers.map((entry) => (
              <RailRow
                key={entry.addr}
                selected={entry.addr === peer?.addr}
                onToggle={() => dispatch({ type: 'peer', peer: entry })}
                title={entry.name}
                meta={`${entry.addr}:${entry.session_port}`}
                reading={formatBytes(entry.free_bytes)}
              />
            ))}
          </ul>
        ) : null}
      </Port>
    </div>
  );
}

/** 内容阶段：来源页签与条目 */
function PickStage({
  state,
  dispatch,
  run,
  iface,
  pairing,
}: {
  state: AppState;
  dispatch: Dispatch;
  run: Run;
  iface: string | null;
  pairing: string | null;
}) {
  const scan = () =>
    run(async () => {
      if (state.source === 'games') {
        dispatch({ type: 'games', games: await api.scanGames() });
        return;
      }
      const entries = await api.listLocal(state.cwd);
      dispatch({ type: 'entries', entries });
    });

  const enter = (entry: LocalEntry) =>
    run(async () => {
      if (entry.is_dir) {
        dispatch({ type: 'cwd', cwd: entry.path });
        dispatch({ type: 'entries', entries: await api.listLocal(entry.path) });
      } else {
        dispatch({ type: 'toggle-pick', path: entry.path });
      }
    });

  const parent = state.cwd
    ? state.cwd.replace(/[\\/][^\\/]*$/, '')
    : null;

  return (
    <div className="grid gap-6 lg:grid-cols-[1fr_1fr]">
      <section className="border border-panel-edge bg-panel-face">
        <header className="flex items-center justify-between gap-3 border-b border-panel-edge px-3 py-2">
          <div className="flex gap-1">
            <Button
              size="sm"
              variant={state.source === 'games' ? 'primary' : 'quiet'}
              onClick={() => dispatch({ type: 'source', source: 'games' })}
            >
              <Gamepad2 className="size-3.5" /> 游戏
            </Button>
            <Button
              size="sm"
              variant={state.source === 'files' ? 'primary' : 'quiet'}
              onClick={() => dispatch({ type: 'source', source: 'files' })}
            >
              <HardDrive className="size-3.5" /> 文件夹
            </Button>
          </div>
          <Button size="sm" onClick={() => void scan()} disabled={state.busy}>
            {state.busy ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
            {state.source === 'games' ? '扫描游戏库' : '列出目录'}
          </Button>
        </header>
        {state.source === 'files' ? (
          <div className="flex items-center gap-2 border-b border-panel-edge px-3 py-2">
            <Button
              size="sm"
              variant="quiet"
              disabled={!parent}
              onClick={() =>
                void run(async () => {
                  dispatch({ type: 'cwd', cwd: parent });
                  dispatch({ type: 'entries', entries: await api.listLocal(parent) });
                })
              }
            >
              <ArrowUp className="size-3.5" /> 上层
            </Button>
            <Input value={state.cwd ?? ''} readOnly placeholder="选择盘符或上层目录" />
          </div>
        ) : null}
        <ul className="max-h-[46vh] overflow-y-auto">
          {state.source === 'games'
            ? state.games.map((game) => (
                <RailRow
                  key={game.id}
                  selected={state.picked.includes(game.install_dir)}
                  onToggle={() => dispatch({ type: 'toggle-pick', path: game.install_dir })}
                  title={game.name}
                  meta={`${game.platform} · ${game.install_dir}`}
                  reading={formatBytes(game.size_bytes)}
                />
              ))
            : state.entries.map((entry) => (
                <RailRow
                  key={entry.path}
                  selected={state.picked.includes(entry.path)}
                  onToggle={() => void enter(entry)}
                  title={entry.is_dir ? `${entry.name}\\` : entry.name}
                  meta={entry.path}
                  reading={entry.is_dir ? '目录' : formatBytes(entry.bytes)}
                  state={entry.is_dir ? <FolderUp className="size-3.5 text-ink-faint" /> : null}
                />
              ))}
        </ul>
      </section>

      <section className="flex flex-col gap-4">
        <div className="border border-panel-edge bg-panel-face px-4 py-3">
          <p className="label">待发清单</p>
          <p className="reading mt-2 text-[22px]">{formatBytes(pickedBytes(state))}</p>
          <p className="text-[13px] text-ink-dim">
            共 {state.picked.length} 项 · 发给 {state.peer?.name ?? '未选择对端'}
          </p>
        </div>
        <ul className="max-h-[32vh] overflow-y-auto border border-panel-edge">
          {state.picked.length === 0 ? (
            <li className="px-3 py-2 text-[13px] text-ink-faint">还没有选中内容</li>
          ) : (
            state.picked.map((path) => (
              <RailRow
                key={path}
                selected
                onToggle={() => dispatch({ type: 'toggle-pick', path })}
                title={path.split(/[\\/]/).pop() ?? path}
                meta={path}
                state={<X className="size-3.5 text-ink-faint" />}
              />
            ))
          )}
        </ul>
        <div className="flex flex-wrap items-center gap-3">
          <Button
            variant="primary"
            size="lg"
            disabled={!canAdvance(state) || state.busy}
            onClick={() =>
              void run(async () => {
                const peer = state.peer;
                if (!peer) {
                  throw new Error('先在连接阶段接上对端');
                }
                dispatch({ type: 'stage', stage: 'transfer' });
                dispatch({ type: 'running', running: true });
                const result = await api.startSend(peer.addr, state.picked, pairing, iface);
                dispatch({ type: 'send-result', result });
                if (!result.approved) {
                  dispatch({ type: 'notice', message: result.message });
                }
              })
            }
          >
            发送给对端
          </Button>
          <Button variant="quiet" onClick={() => dispatch({ type: 'clear-picks' })} disabled={state.picked.length === 0}>
            清空
          </Button>
          <p className="text-[12px] text-ink-faint">对端会看到请求，选定目标文件夹并同意之后才开始搬</p>
        </div>
      </section>
    </div>
  );
}

/** 传输阶段：队列、进度与传入请求 */
function TransferStage({
  state,
  dispatch,
  run,
  iface,
  pairing,
}: {
  state: AppState;
  dispatch: Dispatch;
  run: Run;
  iface: string | null;
  pairing: string | null;
}) {
  const progress = state.progress;
  const incoming = state.incoming;

  return (
    <div className="flex flex-col gap-5">
      {incoming ? (
        <IncomingPanel state={state} dispatch={dispatch} run={run} incoming={incoming} iface={iface} pairing={pairing} />
      ) : null}

      {state.listening && !incoming ? (
        <div className="flex flex-wrap items-center gap-3 border border-panel-edge bg-panel-face px-4 py-3">
          <Lamp state="ready" />
          <span className="text-[13px] text-ink-dim">
            正在等待接收 · {state.listening.addr}:{state.listening.port}
            {state.listening.code ? ` · 配对码 ${state.listening.code}` : ''}
          </span>
          <Button size="sm" variant="quiet" onClick={() => void run(async () => {
            await api.stopListen();
            dispatch({ type: 'listening', listening: null });
          })}>
            停止等待
          </Button>
        </div>
      ) : null}

      <section className="border border-panel-edge">
        <header className="flex items-center justify-between gap-3 border-b border-panel-edge bg-panel-face px-4 py-2">
          <span className="label">本次搬运</span>
          <span className="reading text-[12px] text-ink-dim">
            {state.sendResult?.approved === false
              ? '对端拒绝了这次请求'
              : state.running
                ? '传输中'
                : state.finished
                  ? '已完成'
                  : '等待对端同意'}
          </span>
        </header>
        <div className="px-4 py-3">
          {state.sendResult?.approved ? (
            <p className="text-[13px] text-ink-dim">
              对端已同意，落到 <span className="readi text-ink">{state.sendResult.dest}</span>
            </p>
          ) : null}
          <div className="mt-3 h-2 w-full bg-panel-hole">
            <div
              className="h-full bg-lamp-flow transition-[width] duration-200"
              style={{ width: `${percent(progress?.bytes_done ?? 0, progress?.bytes_total ?? 0).toFixed(1)}%` }}
            />
          </div>
          <div className="reading mt-2 flex flex-wrap gap-x-6 gap-y-1 text-[13px] text-ink-dim">
            <span>{formatBytes(progress?.bytes_done ?? 0)} / {formatBytes(progress?.bytes_total ?? 0)}</span>
            <span>{formatRate(progress?.bytes_per_sec ?? 0)}</span>
            <span>剩余 {formatEta(progress?.bytes_done ?? 0, progress?.bytes_total ?? 0, progress?.bytes_per_sec ?? 0)}</span>
            {state.running ? <span>已下发 {formatBytes(state.sentBytes)}</span> : null}
          </div>
          {state.finished ? (
            <p className="mt-3 text-[13px] text-ink-dim">
              已落到 <span className="readi text-ink">{state.finished.root}</span>
              {state.finished.claim_files.length > 0
                ? ` · 已写认领文件 ${state.finished.claim_files.length} 个`
                : ' · 按启动器的验证完整性收尾'}
            </p>
          ) : null}
        </div>
      </section>
    </div>
  );
}

/** 传入请求：来源、清单、目标文件夹与同意 */
function IncomingPanel({
  state,
  dispatch,
  run,
  incoming,
  iface,
  pairing,
}: {
  state: AppState;
  dispatch: Dispatch;
  run: Run;
  incoming: IncomingEvent;
  iface: string | null;
  pairing: string | null;
}) {
  return (
    <section className="border border-lamp-idle bg-panel-face">
      <header className="flex flex-wrap items-center gap-3 border-b border-panel-edge px-4 py-2">
        <Lamp state="idle" />
        <span className="label">传入请求</span>
        <span className="text-[13px]">
          {incoming.sender_name} 想送来 {formatBytes(incoming.total_bytes)}，共 {incoming.items.length} 项
        </span>
        <span className="reading ml-auto text-[12px] text-ink-faint">{incoming.from}</span>
      </header>
      <ul className="max-h-[26vh] overflow-y-auto">
        {incoming.items.map((item) => (
          <RailRow
            key={item.name}
            selected={false}
            title={item.name}
            meta={item.is_dir ? '文件夹' : '文件'}
            reading={formatBytes(item.bytes)}
          />
        ))}
      </ul>
      <div className="flex flex-wrap items-center gap-3 border-t border-panel-edge px-4 py-3">
        <Input
          value={state.incomingDest}
          onChange={(event) => dispatch({ type: 'incoming-dest', value: event.target.value })}
          placeholder="目标文件夹，例如 D:\\Games"
          className="max-w-md"
        />
        <Button
          size="sm"
          onClick={() =>
            void run(async () => {
              const dest = state.incomingDest.trim();
              if (!dest) {
                throw new Error('先填目标文件夹');
              }
              await api.respondRequest(incoming.id, true, dest);
              dispatch({ type: 'incoming', incoming: null });
              dispatch({ type: 'running', running: true });
              dispatch({ type: 'progress', progress: null });
              const peer = incoming.from;
              const summary = await api.startRecv(peer, incoming.want, dest, null, pairing, true, true);
              dispatch({ type: 'finished', summary });
            })
          }
          variant="primary"
        >
          <Check className="size-3.5" /> 同意传输
        </Button>
        <Button
          size="sm"
          variant="quiet"
          onClick={() =>
            void run(async () => {
              await api.respondRequest(incoming.id, false, null);
              dispatch({ type: 'incoming', incoming: null });
              dispatch({ type: 'notice', message: '已拒绝这次传输' });
            })
          }
        >
          <X className="size-3.5" /> 拒绝
        </Button>
        <span className="text-[12px] text-ink-faint">
          同意之前不会落盘；{iface ? `本机 ${iface}` : ''}
        </span>
      </div>
    </section>
  );
}

/** 底部状态条：此刻只写一件事，并承载唯一的主操作 */
function NextAction({
  state,
  dispatch,
  run,
  iface,
  pairing,
}: {
  state: AppState;
  dispatch: Dispatch;
  run: Run;
  iface: string | null;
  pairing: string | null;
}) {
  if (state.error) {
    return (
      <>
        <Lamp state="fault" />
        <span className="text-lamp-fault">{state.error}</span>
        <Button size="sm" variant="quiet" className="ml-auto" onClick={() => dispatch({ type: 'error', message: null })}>
          知道了
        </Button>
      </>
    );
  }
  if (state.notice) {
    return (
      <>
        <Lamp state="idle" />
        <span>{state.notice}</span>
        <Button size="sm" variant="quiet" className="ml-auto" onClick={() => dispatch({ type: 'notice', message: null })}>
          知道了
        </Button>
      </>
    );
  }
  if (state.stage === 'connect') {
    return (
      <>
        <Lamp state={connected(state) ? 'ready' : 'idle'} />
        <span>{connected(state) ? `已接上 ${state.peer?.name}` : '把跳线接到对端：点「找对端」'}</span>
        <Button
          size="sm"
          variant="primary"
          className="ml-auto"
          disabled={!canAdvance(state)}
          onClick={() => dispatch({ type: 'stage', stage: 'pick' })}
        >
          去选内容
        </Button>
      </>
    );
  }
  if (state.stage === 'pick') {
    return (
      <>
        <Lamp state={state.picked.length > 0 ? 'ready' : 'idle'} />
        <span>
          {state.picked.length > 0
            ? `已选 ${state.picked.length} 项，${formatBytes(pickedBytes(state))}`
            : '选要搬的游戏，或切到「文件夹」挑任意文件'}
        </span>
        <Button size="sm" variant="quiet" className="ml-auto" onClick={() => dispatch({ type: 'stage', stage: 'connect' })}>
          回到连接
        </Button>
      </>
    );
  }
  return (
    <>
      <Lamp state={state.running ? 'flow' : state.finished ? 'ready' : 'idle'} />
      <span>
        {state.running
          ? '正在搬运，中断了也没关系，重新发起只补没传完的部分'
          : state.finished
            ? '这一单完成了'
            : '等待对端处理请求'}
      </span>
      <Button
        size="sm"
        variant="quiet"
        className="ml-auto"
        onClick={() =>
          void run(async () => {
            if (state.running) {
              await api.cancelRecv();
            }
            dispatch({ type: 'reset' });
            dispatch({ type: 'stage', stage: 'connect' });
          })
        }
      >
        {state.running ? '取消并回到连接' : '再搬一次'}
      </Button>
      <span className="hidden">{pairing ?? ''}{iface ?? ''}</span>
    </>
  );
}
