// 三阶段单窗口：连接 → 选择内容 → 传输
//
// 界面只做渲染与状态推进，文件系统与网络全部交给 Rust 侧命令

import { useCallback, useEffect, useReducer, useState } from 'react';
import {
  ArrowUp,
  Check,
  FolderUp,
  Gamepad2,
  HardDrive,
  PlugZap,
  RefreshCw,
  X,
} from 'lucide-react';

import { Lamp, LinkRail, Port, RailRow, StatusStrip, type LampState } from './components/Panel';
import { Button } from './components/ui/button';
import { Input } from './components/ui/input';
import * as api from './lib/api';
import { formatBytes, formatEta, formatRate, percent, sameSubnet, spaceAdvice } from './lib/format';
import {
  canAdvance,
  connected,
  initialState,
  pickedBytes,
  pickedItems,
  pickedPlatform,
  reduce,
  rowState,
  transferComplete,
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

  // 启动：读网络状态与默认目标目录
  useEffect(() => {
    void (async () => {
      dispatch({ type: 'busy', busy: true });
      try {
        dispatch({ type: 'network', network: await api.networkStatus() });
      } catch (error) {
        dispatch({ type: 'error', message: api.describeError(error) });
      }
    })();
    void (async () => {
      try {
        const dest = await api.defaultDest();
        if (dest) {
          dispatch({ type: 'incoming-dest', value: dest });
        }
      } catch {
        // 没有默认目录时留空，由用户自己填
      }
    })();
  }, []);

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
                aria-current={active ? 'step' : undefined}
                onClick={() => dispatch({ type: 'stage', stage: stage.id })}
                className={[
                  'label border border-b-0 px-4 pt-2 pb-2.5',
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
        <span
          className="reading ml-auto mb-3 max-w-[40ch] truncate self-center text-[12px] text-ink-faint"
          title={state.network?.address ?? undefined}
        >
          {state.network?.address ?? '正在读取网络'}
        </span>
      </header>

      <main className="min-h-0 min-w-0 overflow-x-hidden overflow-y-auto px-5 py-5">
        {state.stage === 'connect' ? <ConnectStage state={state} dispatch={dispatch} run={run} iface={iface} /> : null}
        {state.stage === 'pick' ? (
          <PickStage state={state} dispatch={dispatch} run={run} iface={iface} pairing={pairing} />
        ) : null}
        {state.stage === 'transfer' ? (
          <TransferStage state={state} dispatch={dispatch} run={run} iface={iface} pairing={pairing} />
        ) : null}
      </main>

      <StatusStrip alert={state.error !== null}>
        <NextAction state={state} dispatch={dispatch} run={run} />
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
    <div className="grid h-full min-h-[440px] gap-6 md:grid-cols-[1.15fr_0.85fr_1.15fr]">
      <Port
        label="本机"
        name={network?.nic_name ?? (state.busy ? '正在检测' : '未检测到网卡')}
        endpoint={network?.address ?? '—'}
        lamp={network === null ? 'idle' : network.address ? 'ready' : 'fault'}
      >
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            disabled={state.busy}
            onClick={() => void run(async () => dispatch({ type: 'network', network: await api.networkStatus() }))}
          >
            <RefreshCw className="size-3.5" /> 重新检测
          </Button>
          <Button
            size="sm"
            disabled={state.busy}
            onClick={() =>
              void run(async () => {
                const address = await api.setupLink(1);
                dispatch({ type: 'notice', message: `已配置直连地址 ${address}` });
                dispatch({ type: 'network', network: await api.networkStatus() });
              })
            }
          >
            <PlugZap className="size-3.5" /> 本机用 .1
          </Button>
          <Button
            size="sm"
            disabled={state.busy}
            onClick={() =>
              void run(async () => {
                const address = await api.setupLink(2);
                dispatch({ type: 'notice', message: `已配置直连地址 ${address}` });
                dispatch({ type: 'network', network: await api.networkStatus() });
              })
            }
          >
            本机用 .2
          </Button>
          <Button
            size="sm"
            variant="quiet"
            disabled={state.busy}
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
        <p className="text-[12px] text-ink-dim">{network?.hint ?? '正在读取网络'}</p>
        <label className="flex items-center gap-3 text-[12px] text-ink-dim">
          <span className="label shrink-0">配对码</span>
          <Input
            value={state.pairing}
            inputMode="numeric"
            maxLength={6}
            placeholder="留空即不校验"
            onChange={(event) => dispatch({ type: 'pairing', value: event.target.value })}
            className="max-w-[10rem]"
          />
        </label>
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
              <RefreshCw className="size-3.5" />
              {state.busy ? '正在找' : '找对端'}
            </Button>
          </div>
        )}
        {peer && !sameSubnet(network?.address ?? null, peer.addr) ? (
          <p className="min-w-0 break-all text-[12px] text-lamp-standby">
            本机 {network?.address} 与对端 {peer.addr} 不在同一个网段：广播能互相看见，但数据传不过去。一端点「本机用 .1」、另一端点「本机用 .2」，或把两台机器接到同一个路由器上
          </p>
        ) : null}
        {state.peers.length > 0 ? (
          <ul className="border border-panel-edge">
            {state.peers.map((entry) => (
              <RailRow
                key={entry.addr}
                selected={entry.addr === peer?.addr}
                onSelect={() => dispatch({ type: 'peer', peer: entry })}
                selectLabel={`接上 ${entry.name}`}
                title={entry.name}
                meta={`${entry.addr}:${entry.session_port}`}
                reading={entry.free_bytes > 0 ? `可用 ${formatBytes(entry.free_bytes)}` : '空间未知'}
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
      dispatch({ type: 'cwd', cwd: entry.path });
      dispatch({ type: 'entries', entries: await api.listLocal(entry.path) });
    });

  const parent = state.cwd
    ? state.cwd.replace(/[\\/][^\\/]*$/, '')
    : null;

  return (
    <div className="grid min-w-0 gap-6 md:grid-cols-[1fr_1fr]">
      <section className="min-w-0 border border-panel-edge bg-panel-face">
        <header className="flex items-center justify-between gap-3 border-b border-panel-edge px-4 py-2">
          <div className="flex gap-1" role="group" aria-label="内容来源">
            <Button
              size="sm"
              variant={state.source === 'games' ? 'key' : 'quiet'}
              aria-pressed={state.source === 'games'}
              className={state.source === 'games' ? 'border-lamp-flow text-lamp-flow' : ''}
              onClick={() => dispatch({ type: 'source', source: 'games' })}
            >
              <Gamepad2 className="size-3.5" /> 游戏
            </Button>
            <Button
              size="sm"
              variant={state.source === 'files' ? 'key' : 'quiet'}
              aria-pressed={state.source === 'files'}
              className={state.source === 'files' ? 'border-lamp-flow text-lamp-flow' : ''}
              onClick={() => dispatch({ type: 'source', source: 'files' })}
            >
              <HardDrive className="size-3.5" /> 文件夹
            </Button>
          </div>
          <Button size="sm" onClick={() => void scan()} disabled={state.busy}>
            <RefreshCw className="size-3.5" />
            {state.source === 'games' ? '扫描游戏库' : '列出目录'}
          </Button>
        </header>
        {state.source === 'files' ? (
          <div className="flex items-center gap-2 border-b border-panel-edge px-4 py-2">
            <Button
              size="sm"
              variant="quiet"
              disabled={!parent || state.busy}
              onClick={() =>
                void run(async () => {
                  dispatch({ type: 'cwd', cwd: parent });
                  dispatch({ type: 'entries', entries: await api.listLocal(parent) });
                })
              }
            >
              <ArrowUp className="size-3.5" /> 上层
            </Button>
            <Input
              value={state.cwd ?? ''}
              readOnly
              aria-label="当前目录"
              placeholder="点「列出目录」从盘符开始，或点上层返回"
            />
          </div>
        ) : null}
        <ul className="max-h-[46vh] overflow-y-auto">
          {state.source === 'games'
            ? state.games.map((game) => (
                <RailRow
                  key={game.id}
                  selected={state.picked.includes(game.install_dir)}
                  onSelect={() => dispatch({ type: 'toggle-pick', path: game.install_dir })}
                  selectLabel={`选中 ${game.name}`}
                  title={game.name}
                  meta={`${game.platform} · ${game.install_dir}`}
                  reading={formatBytes(game.size_bytes)}
                />
              ))
            : state.entries.map((entry) => (
                <RailRow
                  key={entry.path}
                  selected={state.picked.includes(entry.path)}
                  onSelect={() => dispatch({ type: 'toggle-pick', path: entry.path })}
                  onActivate={entry.is_dir ? () => void enter(entry) : undefined}
                  selectLabel={`选中 ${entry.name}`}
                  title={entry.is_dir ? `${entry.name}\\` : entry.name}
                  meta={entry.path}
                  reading={entry.is_dir ? '文件夹' : formatBytes(entry.bytes)}
                  state={entry.is_dir ? <FolderUp className="size-3.5 text-ink-faint" /> : null}
                />
              ))}
          {state.source === 'games' && state.games.length === 0 ? (
            <li className="px-4 py-3 text-[13px] text-ink-dim">
              还没有读到游戏库，点右上「扫描游戏库」；也可以切到「文件夹」发任意内容
            </li>
          ) : null}
          {state.source === 'files' && state.entries.length === 0 ? (
            <li className="px-4 py-3 text-[13px] text-ink-dim">
              这一层没有可选项，点「上层」返回，或点「列出目录」从盘符开始
            </li>
          ) : null}
        </ul>
      </section>

      <section className="flex min-w-0 flex-col gap-6">
        <div className="border border-panel-edge bg-panel-face px-4 py-3">
          <p className="label">待发清单</p>
          <p className="reading mt-2 text-[22px]">{formatBytes(pickedBytes(state))}</p>
          <p className="text-[13px] text-ink-dim">
            共 {state.picked.length} 项 · 发给 {state.peer?.name ?? '未选择对端'}
          </p>
        </div>
        <ul className="max-h-[32vh] overflow-y-auto border border-panel-edge">
          {state.picked.length === 0 ? (
            <li className="px-4 py-3 text-[13px] text-ink-dim">还没有选中内容</li>
          ) : (
            state.picked.map((path) => (
              <RailRow
                key={path}
                selected
                onSelect={() => dispatch({ type: 'toggle-pick', path })}
                selectLabel={`移出 ${path}`}
                title={path.split(/[\\/]/).pop() ?? path}
                meta={path}
                state={<X className="size-3.5 text-ink-faint" />}
              />
            ))
          )}
        </ul>
        <div className="flex min-w-0 flex-wrap items-center gap-3">
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
                dispatch({ type: 'transfer-plan', role: 'send', items: pickedItems(state) });
                const result = await api.startSend(peer.addr, state.picked, pairing, iface, pickedPlatform(state));
                if (result.approved) {
                  dispatch({ type: 'send-result', result });
                  dispatch({ type: 'notice', message: `对端已同意，落到 ${result.dest ?? '对端选的目录'}` });
                  return;
                }
                dispatch({ type: 'send-result', result });
                dispatch({ type: 'error', message: result.message });
              })
            }
          >
            发送给对端
          </Button>
          <Button variant="quiet" onClick={() => dispatch({ type: 'clear-picks' })} disabled={state.picked.length === 0}>
            清空
          </Button>
          <p className="min-w-0 break-words text-[12px] text-ink-dim">对端会看到请求，选定目标文件夹并同意之后才开始搬</p>
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
  const done = transferComplete(state);
  const row = rowState(state);

  return (
    <div className="flex flex-col gap-6">
      {incoming ? (
        <IncomingPanel state={state} dispatch={dispatch} run={run} incoming={incoming} iface={iface} pairing={pairing} />
      ) : null}

      {state.listening && !incoming ? (
        <div className="flex min-w-0 flex-wrap items-center gap-3 border border-panel-edge bg-panel-face px-4 py-3">
          <Lamp state={state.listening.warning ? 'fault' : 'ready'} />
          <span className="reading text-[13px] text-ink-dim">
            正在等待接收 · {state.listening.addr}:{state.listening.port}
            {state.listening.code ? ` · 配对码 ${state.listening.code}` : ''}
          </span>
          {state.listening.warning ? (
            <span className="min-w-0 break-all text-[13px] text-lamp-fault">{state.listening.warning}</span>
          ) : null}
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
              : done
                ? '已完成'
                : state.running
                  ? '传输中'
                  : state.role === 'receive'
                    ? '等待你同意'
                    : state.sendResult?.approved
                      ? '对端已同意'
                      : '等待对端同意'}
          </span>
        </header>
        {state.transferItems.length > 0 ? (
          <ul className="border-b border-panel-edge">
            {state.transferItems.map((item) => (
              <RailRow
                key={item.name}
                selected={false}
                title={item.name}
                meta={
                  state.role === 'receive'
                    ? '来自对端'
                    : item.source ?? `发往 ${state.sendResult?.dest ?? '对端'}`
                }
                reading={formatBytes(item.bytes)}
                state={
                  <span
                    key={`${item.name}-${row}`}
                    className={[
                      'label flap shrink-0',
                      row === 'flowing' ? 'text-lamp-flow' : '',
                      row === 'done' ? 'text-lamp-ready' : '',
                    ].join(' ')}
                  >
                    {row === 'done' ? '已完成' : row === 'flowing' ? '传输中' : '等待'}
                  </span>
                }
              />
            ))}
          </ul>
        ) : null}
        <div className="px-4 py-3">
          {state.sendResult?.approved ? (
            <p className="text-[13px] text-ink-dim">
              对端已同意，落到{' '}
              <span className="reading break-all text-ink">{state.sendResult.dest}</span>
            </p>
          ) : null}
          <div className="mt-3 h-2 w-full bg-panel-hole">
            <div
              className="h-full bg-lamp-flow transition-[width] duration-200"
              style={{ width: `${percent(progress?.bytes_done ?? 0, progress?.bytes_total ?? 0).toFixed(1)}%` }}
            />
          </div>
          <div className="reading mt-2 flex flex-wrap gap-x-6 gap-y-1 text-[13px] text-ink-dim">
            <span key={progress?.bytes_done ?? 0} className="flap">
              {formatBytes(progress?.bytes_done ?? 0)} / {formatBytes(progress?.bytes_total ?? 0)}
            </span>
            <span>{formatRate(progress?.bytes_per_sec ?? 0)}</span>
            <span>剩余 {formatEta(progress?.bytes_done ?? 0, progress?.bytes_total ?? 0, progress?.bytes_per_sec ?? 0)}</span>
          </div>
          {state.finished ? (
            <p className="mt-3 text-[13px] text-ink-dim">
              已落到 <span className="reading break-all text-ink">{state.finished.root}</span>
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
  const [space, setSpace] = useState<string>('');

  // 目标目录一变就查一次可用空间，够不够在同意之前就说清楚
  useEffect(() => {
    const dest = state.incomingDest.trim();
    if (dest.length === 0) {
      setSpace('');
      return;
    }
    let alive = true;
    void api
      .freeSpace(dest)
      .then((free) => {
        if (alive) {
          setSpace(spaceAdvice(free, incoming.total_bytes));
        }
      })
      .catch(() => {
        if (alive) {
          setSpace('');
        }
      });
    return () => {
      alive = false;
    };
  }, [state.incomingDest, incoming.total_bytes]);

  return (
    <section className="border border-lamp-idle bg-panel-face">
      <header className="flex flex-wrap items-center gap-3 border-b border-panel-edge px-4 py-2">
        <Lamp state="idle" />
        <span className="label">传入请求</span>
        <span className="text-[13px]">
          {incoming.sender_name} 想送来 {formatBytes(incoming.total_bytes)}，共 {incoming.items.length} 项
          {incoming.platform ? ` · ${incoming.platform}` : ''}
        </span>
        <span className="reading ml-auto truncate text-[12px] text-ink-faint">{incoming.from}</span>
      </header>
      <ul className="max-h-[26vh] overflow-y-auto">
        {incoming.items.map((item, index) => (
          <RailRow
            key={`${item.name}-${index}`}
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
          aria-label="目标文件夹"
          onChange={(event) => dispatch({ type: 'incoming-dest', value: event.target.value })}
          placeholder="目标文件夹，例如 D:\\Games"
          className="max-w-md"
        />
        <Button
          size="sm"
          disabled={state.busy || state.incomingDest.trim().length === 0}
          onClick={() =>
            void run(async () => {
              const dest = state.incomingDest.trim();
              if (!dest) {
                throw new Error('先填目标文件夹');
              }
              await api.respondRequest(incoming.id, true, dest);
              dispatch({ type: 'incoming', incoming: null });
              dispatch({ type: 'transfer-plan', role: 'receive', items: incoming.items });
              dispatch({ type: 'running', running: true });
              dispatch({ type: 'progress', progress: null });
              dispatch({ type: 'notice', message: '已同意，开始搬运' });
              const peer = incoming.from;
              const summary = await api.startRecv(peer, incoming.want, dest, incoming.platform, pairing, true, true);
              dispatch({ type: 'finished', summary });
              dispatch({ type: 'notice', message: null });
            })
          }
          variant="primary"
        >
          <Check className="size-3.5" /> 同意传输
        </Button>
        <Button
          size="sm"
          variant="quiet"
          disabled={state.busy}
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
        <span className="min-w-0 flex-1 break-all text-[12px] text-ink-dim">
          {space || `同意之前不会落盘${iface ? ` · 本机 ${iface}` : ''}`}
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
}: {
  state: AppState;
  dispatch: Dispatch;
  run: Run;
}) {
  const done = transferComplete(state);

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
      <Lamp state={done ? 'ready' : state.running ? 'flow' : 'idle'} />
      <span>
        {done
          ? '这一单完成了'
          : state.running
            ? '正在搬运，中断了也没关系，重新发起只补没传完的部分'
            : state.role === 'receive'
              ? `对端想送来 ${state.transferItems.length} 项，选好目标文件夹后同意`
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
    </>
  );
}
