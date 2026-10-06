import { describe, expect, it } from 'vitest';

import {
  endpointText,
  formatBytes,
  formatDuration,
  formatEta,
  formatRate,
  percent,
  sameSubnet,
  spaceAdvice,
} from '../src/lib/format';

describe('formatBytes', () => {
  it('按 1024 进制换算', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(1024)).toBe('1.0 KiB');
    expect(formatBytes(1536)).toBe('1.5 KiB');
    expect(formatBytes(1024 * 1024)).toBe('1.0 MiB');
    expect(formatBytes(60 * 1024 * 1024 * 1024)).toBe('60.0 GiB');
  });

  it('非法输入回落到 0 B', () => {
    expect(formatBytes(Number.NaN)).toBe('0 B');
    expect(formatBytes(-1)).toBe('0 B');
  });
});

describe('formatRate', () => {
  it('速率为零时提示待测', () => {
    expect(formatRate(0)).toBe('待测');
    expect(formatRate(Number.POSITIVE_INFINITY)).toBe('待测');
  });

  it('正常速率带单位', () => {
    expect(formatRate(1024 * 1024)).toBe('1.0 MiB/s');
  });
});

describe('formatDuration', () => {
  it('按大小选择单位', () => {
    expect(formatDuration(45)).toBe('45 秒');
    expect(formatDuration(150)).toBe('2 分 30 秒');
    expect(formatDuration(7200)).toBe('2 小时 0 分');
  });

  it('非法输入返回未知', () => {
    expect(formatDuration(-3)).toBe('未知');
    expect(formatDuration(Number.NaN)).toBe('未知');
  });
});

describe('formatEta', () => {
  it('按剩余量与速率估算', () => {
    expect(formatEta(0, 2048, 1024)).toBe('2 秒');
    expect(formatEta(1024, 2048, 0)).toBe('待测');
  });
});

describe('percent', () => {
  it('总量未知时不画进度，避免出现满格假象', () => {
    expect(percent(0, 0)).toBe(0);
    expect(percent(0, Number.NaN)).toBe(0);
  });

  it('夹在 0 到 100 之间', () => {
    expect(percent(50, 100)).toBe(50);
    expect(percent(200, 100)).toBe(100);
    expect(percent(-5, 100)).toBe(0);
  });
});

describe('spaceAdvice', () => {
  it('空间充足时给出剩余量', () => {
    expect(spaceAdvice(4096, 2048)).toContain('空间够用');
  });

  it('空间不足时给出缺口', () => {
    const advice = spaceAdvice(1024, 4096);
    expect(advice).toContain('还差');
  });

  it('无需空间时不提示', () => {
    expect(spaceAdvice(1024, 0)).toBe('');
  });
});

describe('sameSubnet', () => {
  it('前三位相同才算同网段', () => {
    expect(sameSubnet('192.168.88.1', '192.168.88.2')).toBe(true);
    expect(sameSubnet('192.168.88.1', '10.10.10.1')).toBe(false);
  });

  it('带端口也能比较，缺值时按同网段处理', () => {
    expect(sameSubnet('192.168.88.1', '192.168.88.2:27102')).toBe(true);
    expect(sameSubnet(null, '10.10.10.1')).toBe(true);
    expect(sameSubnet('不是地址', '10.10.10.1')).toBe(true);
  });
});

describe('endpointText', () => {
  it('端口为零时只显示地址', () => {
    expect(endpointText('192.168.88.2', 0)).toBe('192.168.88.2');
    expect(endpointText('192.168.88.2', 27101)).toBe('192.168.88.2:27101');
  });
});
