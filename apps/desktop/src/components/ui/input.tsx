import type { ComponentProps } from 'react';

import { cn } from '../../lib/utils';

/** 面板上的输入孔 */
export function Input({ className, ...props }: ComponentProps<'input'>) {
  return (
    <input
      className={cn(
        'h-9 w-full min-w-0 border border-panel-rim bg-panel-hole px-3 text-[14px] text-ink',
        'placeholder:text-ink-faint focus-visible:border-lamp-flow',
        'disabled:cursor-not-allowed disabled:opacity-40',
        className,
      )}
      {...props}
    />
  );
}
