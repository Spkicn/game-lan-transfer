// 纯格式化函数：容量、速率、时长与空间建议

const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB'] as const;

/** 人类可读的容量 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return '0 B';
  }
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const label = UNITS[unit] ?? 'B';
  return unit === 0 ? `${Math.round(value)} ${label}` : `${value.toFixed(1)} ${label}`;
}

/** 传输速率 */
export function formatRate(bytesPerSec: number): string {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec <= 0) {
    return '待测';
  }
  return `${formatBytes(bytesPerSec)}/s`;
}

/** 时长 */
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) {
    return '未知';
  }
  const total = Math.round(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = total % 60;
  if (hours > 0) {
    return `${hours} 小时 ${minutes} 分`;
  }
  if (minutes > 0) {
    return `${minutes} 分 ${rest} 秒`;
  }
  return `${rest} 秒`;
}

/** 剩余时间 */
export function formatEta(done: number, total: number, rate: number): string {
  if (!Number.isFinite(rate) || rate <= 0) {
    return '待测';
  }
  return formatDuration(Math.max(0, total - done) / rate);
}

/** 完成比例，取值 0–100 */
export function percent(done: number, total: number): number {
  if (!Number.isFinite(total) || total <= 0) {
    return 100;
  }
  const value = (done / total) * 100;
  return Math.min(100, Math.max(0, value));
}

/** 空间预检文案 */
export function spaceAdvice(free: number, need: number): string {
  if (!Number.isFinite(need) || need <= 0) {
    return '';
  }
  if (free >= need) {
    return `空间够用，传完后还剩 ${formatBytes(free - need)}`;
  }
  return `还差 ${formatBytes(need - free)}，先清理目标盘或换一个盘`;
}

/** 链路速率档位，用于端口上的一行读数 */
export function linkSpeedText(free: number): string {
  return free > 0 ? `对端可用 ${formatBytes(free)}` : '对端空间未知';
}

/** 端口标签：把 IPv4 与端口拼成面板读数 */
export function endpointText(addr: string, port: number): string {
  return port > 0 ? `${addr}:${port}` : addr;
}
