// 面板零件：端口、指示灯、跳线与行
//
// 这一层只画配线架上的物件，不承载任何业务判断

import type { ReactNode } from 'react';

import { cn } from '../lib/utils';

import { Checkbox } from './ui/checkbox';

/** 灯的四态 */
export type LampState = 'idle' | 'ready' | 'flow' | 'fault';

const LAMP_CLASS: Record<LampState, string> = {
  idle: 'bg-lamp-idle',
  ready: 'bg-lamp-ready',
  flow: 'bg-lamp-flow',
  fault: 'bg-lamp-fault',
};

/** 状态灯 */
export function Lamp({ state, title }: { state: LampState; title?: string }) {
  return (
    <span
      className={cn('inline-block size-2.5 rounded-full', LAMP_CLASS[state])}
      title={title}
      aria-hidden
    />
  );
}

/** 端口：一块孔位，孔下面挂读数 */
export function Port({
  label,
  name,
  endpoint,
  lamp,
  children,
  slot = false,
}: {
  label: string;
  name: string;
  endpoint: string;
  lamp: LampState;
  children?: ReactNode;
  slot?: boolean;
}) {
  return (
    <section className="flex h-full min-w-0 flex-col gap-3">
      <header className="flex items-center justify-between gap-3">
        <span className="label">{label}</span>
        <Lamp state={lamp} />
      </header>
      <div
        className={cn(
          'flex flex-1 flex-col justify-start gap-2 border px-4 py-4',
          slot ? 'border-dashed border-panel-rim/60 bg-transparent' : 'border-panel-edge bg-panel-face',
        )}
      >
        <p className="truncate text-[19px] font-semibold tracking-tight" title={name}>
          {name}
        </p>
        <p className="reading truncate text-[13px] text-ink-dim" title={endpoint}>
          {endpoint}
        </p>
      </div>
      {/* 两边控制件高度对齐，机位底边才会齐 */}
      <div className="flex min-h-[104px] flex-col gap-3">{children}</div>
    </section>
  );
}

/** 跳线：两端之间只走 45° 与 90° 的折线，横跨整个中间列 */
export function LinkRail({
  state,
  caption,
}: {
  state: LampState;
  caption: string;
}) {
  const stroke =
    state === 'ready' || state === 'flow'
      ? 'var(--color-lamp-flow)'
      : state === 'fault'
        ? 'var(--color-lamp-fault)'
        : 'var(--color-panel-edge)';
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3">
      <svg
        viewBox="0 0 200 60"
        preserveAspectRatio="none"
        className="-mx-6 h-16 w-[calc(100%+3rem)]"
        aria-hidden
      >
        <path
          d="M0 30 H70 L95 8 H130 L155 30 H200"
          fill="none"
          stroke={stroke}
          strokeWidth="2"
          vectorEffect="non-scaling-stroke"
          strokeLinejoin="miter"
          strokeDasharray={state === 'idle' ? '5 6' : undefined}
        />
      </svg>
      <p className="label text-center">{caption}</p>
    </div>
  );
}

/** 面板上的行：选择孔、名字、读数、状态 */
export function RailRow({
  selected,
  onSelect,
  onActivate,
  selectLabel,
  title,
  meta,
  reading,
  state,
  disabled = false,
}: {
  selected: boolean;
  onSelect?: () => void;
  onActivate?: () => void;
  selectLabel?: string;
  title: string;
  meta?: string;
  reading?: string;
  state?: ReactNode;
  disabled?: boolean;
}) {
  return (
    <li
      className={cn(
        'flex min-w-0 items-center gap-3 border-b border-panel-edge/70 px-3 py-2.5 last:border-b-0',
        selected && 'bg-[color-mix(in_oklab,var(--color-lamp-flow)_10%,transparent)]',
        disabled && 'opacity-50',
      )}
    >
      {onSelect ? (
        <Checkbox
          checked={selected}
          label={selectLabel ?? title}
          disabled={disabled}
          onCheckedChange={() => onSelect()}
        />
      ) : (
        <span className="size-4 shrink-0" aria-hidden />
      )}
      <button
        type="button"
        className="min-w-0 flex-1 text-left disabled:cursor-default"
        onClick={onActivate ?? onSelect}
        disabled={(!onActivate && !onSelect) || disabled}
      >
        <span className="block truncate text-[14px]" title={title}>
          {title}
        </span>
        {meta ? (
          <span className="reading block truncate text-[12px] text-ink-faint" title={meta}>
            {meta}
          </span>
        ) : null}
      </button>
      {reading ? (
        <span className="reading shrink-0 text-[13px] text-ink-dim">{reading}</span>
      ) : null}
      {state}
    </li>
  );
}

/** 底部状态条：只写此刻该做的一件事，出现变化时也让读屏知道 */
export function StatusStrip({ children, alert = false }: { children: ReactNode; alert?: boolean }) {
  return (
    <footer
      role={alert ? 'alert' : 'status'}
      aria-live={alert ? 'assertive' : 'polite'}
      className="flex min-h-11 min-w-0 flex-wrap items-center gap-x-4 gap-y-1 border-t border-panel-edge bg-panel-rail px-5 py-2 text-[13px] text-ink-dim"
    >
      {children}
    </footer>
  );
}
