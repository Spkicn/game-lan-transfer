import { Slot } from '@radix-ui/react-slot';
import { cva, type VariantProps } from 'class-variance-authority';
import type { ComponentProps } from 'react';

import { cn } from '../../lib/utils';

/** 面板上的操作件；默认是孔位上的按键，主操作是通电的那一个 */
const buttonVariants = cva(
  'inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-colors duration-100 disabled:pointer-events-none disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-lamp-flow',
  {
    variants: {
      variant: {
        primary:
          'bg-lamp-flow text-panel-hole hover:bg-[color-mix(in_oklab,var(--color-lamp-flow)_88%,white)]',
        key: 'border border-panel-edge bg-panel-face text-ink hover:border-lamp-flow hover:text-lamp-flow',
        quiet: 'text-ink-dim hover:text-ink',
      },
      size: {
        default: 'h-9 px-4 text-sm',
        sm: 'h-8 px-3 text-[13px]',
        lg: 'h-11 px-6 text-[15px]',
      },
    },
    defaultVariants: {
      variant: 'key',
      size: 'default',
    },
  },
);

/** 按钮 */
export function Button({
  className,
  variant,
  size,
  asChild = false,
  ...props
}: ComponentProps<'button'> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean;
  }) {
  const Comp = asChild ? Slot : 'button';
  return <Comp className={cn(buttonVariants({ variant, size }), className)} {...props} />;
}

export { buttonVariants };
