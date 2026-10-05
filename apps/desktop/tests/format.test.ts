import { describe, expect, it } from 'vitest';

import {
  formatBytes,
  formatDuration,
  formatEta,
  formatRate,
  percent,
  spaceAdvice,
} from '../src/format';

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
  it('速率为零时提示计算中', () => {
    expect(formatRate(0)).toBe('计算中');
    expect(formatRate(Number.POSITIVE_INFINITY)).toBe('计算中');
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
    expect(formatEta(1024, 2048, 0)).toBe('计算中');
  });
});

describe('percent', () => {
  it('总数为零视为已完成', () => {
    expect(percent(0, 0)).toBe(100);
  });

  it('夹在 0 到 100 之间', () => {
    expect(percent(50, 100)).toBe(50);
    expect(percent(200, 100)).toBe(100);
    expect(percent(-5, 100)).toBe(0);
  });
});

describe('spaceAdvice', () => {
  it('空间充足时给出剩余量', () => {
    expect(spaceAdvice(4096, 2048)).toContain('空间充足');
  });

  it('空间不足时给出缺口', () => {
    const advice = spaceAdvice(1024, 4096);
    expect(advice).toContain('空间不足');
    expect(advice).toContain('还差');
  });

  it('无需空间时不提示', () => {
    expect(spaceAdvice(1024, 0)).toBe('');
  });
});
