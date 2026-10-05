import * as CheckboxPrimitive from '@radix-ui/react-checkbox';
import { Check } from 'lucide-react';
import type { ComponentProps } from 'react';

import { cn } from '../../lib/utils';

/** 面板上的选择孔：选中即插上 */
export function Checkbox({
  className,
  label,
  ...props
}: ComponentProps<typeof CheckboxPrimitive.Root> & { label?: string }) {
  return (
    <CheckboxPrimitive.Root
      aria-label={label}
      className={cn(
        'peer size-4 shrink-0 border border-panel-rim bg-panel-hole transition-colors',
        'data-[state=checked]:border-lamp-ready data-[state=checked]:bg-lamp-ready',
        'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-lamp-flow',
        'disabled:opacity-40',
        className,
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator className="flex items-center justify-center text-panel-hole">
        <Check className="size-3.5" strokeWidth={3} />
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
}
