import { useCallback, useEffect, useState } from "react";
export type System = {
  fw: string;
  revision: string;
  idf: string;
  mac: string;
  uptime_s: number;
  reset_reason: string;
  sample_count: number;
  heap_free: number;
  heap_total: number;
  heap_min: number;
  heap_largest: number;
  psram_free: number;
  psram_total: number;
  stack_hwm: number;
  flash_total: number;
  app_size: number;
  app_used: number;
  app_part_addr: number;
  temp_c: number | null;
  humid_pct: number | null;
  chip_temp_c: number | null;
  temp_off_c: number;
  humid_off_pct: number;
  battery_mv: number | null;
  usb_plugged: boolean | null;
  wifi_connected: boolean;
  wifi_ssid: string;
  wifi_ip: number[] | null;
  wifi_rssi: number | null;
  unix_secs: number | null;
  clock_source: string;
  sampled_at: number | null;
};
export type Config = Record<string, string | number | boolean>;
export async function api<T>(
  path: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const response = await fetch(path, {
    cache: "no-store",
    signal: signal || AbortSignal.timeout(12000),
    ...(body !== undefined
      ? {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        }
      : {}),
  });
  const text = await response.text();
  let result;
  try {
    result = JSON.parse(text);
  } catch {
    result = null;
  }
  if (!response.ok || result?.ok === false)
    throw Error(result?.error || text || `请求失败 (${response.status})`);
  return result as T;
}
export function usePoll<T>(path: string, interval = 5000, paused = false) {
  const [data, setData] = useState<T | null>(null),
    [error, setError] = useState(""),
    [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision((n) => n + 1), []);
  useEffect(() => {
    if (paused) return;
    let alive = true,
      timer: ReturnType<typeof setTimeout>;
    const controller = new AbortController();
    async function poll() {
      if (!document.hidden) {
        try {
          const value = await api<T>(
            path,
            undefined,
            AbortSignal.any([controller.signal, AbortSignal.timeout(8000)]),
          );
          if (alive) {
            setData(value);
            setError("");
          }
        } catch (e) {
          if (alive)
            setError(e instanceof Error ? e.message : "暂时无法连接设备");
        }
      }
      if (alive) timer = setTimeout(poll, interval);
    }
    void poll();
    return () => {
      alive = false;
      clearTimeout(timer);
      controller.abort();
    };
  }, [path, interval, paused, revision]);
  return { data, error, refresh };
}
export const bytes = (n?: number) =>
  n == null
    ? "—"
    : n >= 1048576
      ? `${(n / 1048576).toFixed(1)} MB`
      : `${(n / 1024).toFixed(1)} KB`;
export function uptime(n?: number) {
  if (n == null) return "—";
  const d = Math.floor(n / 86400),
    h = Math.floor((n % 86400) / 3600),
    m = Math.floor((n % 3600) / 60);
  return d
    ? `${d} 天 ${h} 小时`
    : h
      ? `${h} 小时 ${m} 分钟`
      : `${m} 分钟 ${n % 60} 秒`;
}
export const reading = (n: number | null | undefined, digits = 1) =>
  n == null ? "—" : n.toFixed(digits);
