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
    <section className="flex min-w-0 flex-col gap-3">
      <header className="flex items-center justify-between gap-3">
        <span className="label">{label}</span>
        <Lamp state={lamp} />
      </header>
      <div
        className={cn(
          'border px-4 py-3',
          slot ? 'border-dashed border-panel-edge bg-transparent' : 'border-panel-edge bg-panel-face',
        )}
      >
        <p className="truncate text-[17px] font-semibold tracking-tight">{name}</p>
        <p className="reading mt-1 truncate text-[13px] text-ink-dim">{endpoint}</p>
      </div>
      {children}
    </section>
  );
}

/** 跳线：两端之间只走 45° 与 90° 的折线 */
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
    <div className="flex flex-col items-center justify-center gap-2 px-2">
      <svg viewBox="0 0 140 72" className="h-16 w-full" aria-hidden>
        <path
          d="M4 36 H52 L74 14 H136"
          fill="none"
          stroke={stroke}
          strokeWidth="2"
          strokeLinejoin="miter"
          strokeDasharray={state === 'idle' ? '4 5' : undefined}
        />
        <circle cx="4" cy="36" r="3" fill={stroke} />
        <circle cx="136" cy="14" r="3" fill={stroke} />
      </svg>
      <p className="label text-center">{caption}</p>
    </div>
  );
}

/** 面板上的行：选择孔、名字、读数、状态 */
export function RailRow({
  selected,
  onToggle,
  title,
  meta,
  reading,
  state,
  disabled = false,
}: {
  selected: boolean;
  onToggle?: () => void;
  title: string;
  meta?: string;
  reading?: string;
  state?: ReactNode;
  disabled?: boolean;
}) {
  return (
    <li
      className={cn(
        'flex items-center gap-3 border-b border-panel-edge/70 px-3 py-2.5 last:border-b-0',
        selected && 'bg-[color-mix(in_oklab,var(--color-lamp-flow)_10%,transparent)]',
        disabled && 'opacity-50',
      )}
    >
      {onToggle ? (
        <Checkbox checked={selected} disabled={disabled} onCheckedChange={() => onToggle()} />
      ) : (
        <span className="size-4" aria-hidden />
      )}
      <button
        type="button"
        className="min-w-0 flex-1 text-left disabled:cursor-default"
        onClick={onToggle}
        disabled={!onToggle || disabled}
      >
        <span className="block truncate text-[14px]">{title}</span>
        {meta ? <span className="reading block truncate text-[12px] text-ink-faint">{meta}</span> : null}
      </button>
      {reading ? <span className="reading shrink-0 text-[13px] text-ink-dim">{reading}</span> : null}
      {state}
    </li>
  );
}

/** 底部状态条：只写此刻该做的一件事 */
export function StatusStrip({ children }: { children: ReactNode }) {
  return (
    <footer className="flex min-h-11 flex-wrap items-center gap-x-4 gap-y-1 border-t border-panel-edge bg-panel-rail px-5 py-2 text-[13px] text-ink-dim">
      {children}
    </footer>
  );
}
